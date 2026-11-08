//! Brand kits made and changed by AI assistants (`create_brand_kit`, `update_brand_kit`): the same kit rules and
//! roles as Brand Hub (routes/brandhub.rs), with the kit's images taken from the user's own uploads (put there with
//! `upload_image` or `create_upload_link`), so an assistant can set up a whole brand from a brand book or website.

use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::tools::{Fail, R, find_kit, kit_views, need, opt_str, text, tool_err};
use crate::bearer::BearerUser;
use crate::routes::brandhub::{self as hub, MAX_BRAND_KITS, MAX_GUIDELINES_TOTAL};
use crate::state::AppState;

/// Kit fields an assistant sets, as (argument, kit field); the image lists are handled on their own.
const FIELDS: [(&str, &str); 5] = [("colors", "colors"), ("palettes", "palettes"), ("fonts", "fonts"), ("text_styles", "textStyles"), ("voice", "voice")];
const IMAGE_LISTS: [&str; 3] = ["logos", "graphics", "photos"];

const NEXT: &str = "Check the kit as it now stands. If missingFonts is set, tell the user those font files must be uploaded in Brand Hub (the kit's Fonts), or pick Google Fonts with search_fonts instead. Add more logos, graphics or photos with update_brand_kit { add_logos | add_graphics | add_photos } after uploading them with upload_image or create_upload_link. Design with it by passing its name to create_design as brand_kit.";

async fn audit(s: &AppState, u: &BearerUser, ws: Uuid, action: &str, kit_id: &str, mut meta: Map<String, Value>) -> Result<(), Fail> {
    meta.insert("via".into(), json!("ai"));
    meta.insert("client".into(), json!(u.client_name));
    let e = oc_audit::AuditEvent { actor_id: Some(u.id), workspace_id: Some(ws), action: action.into(), target: Some(kit_id.into()), meta: Some(Value::Object(meta)), ip: None };
    oc_audit::audit_at(&s.pool, e, s.now()).await?;
    Ok(())
}

/// The workspace a new kit goes in: the named team (where the user is a member), or their personal workspace.
async fn target_workspace(s: &AppState, user_id: Uuid, team: Option<&str>) -> Result<Uuid, Fail> {
    let Some(team) = team.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(crate::auth::personal_workspace(s, user_id).await?);
    };
    let teams: Vec<(Uuid, String)> = sqlx::query_as("SELECT w.id, w.name FROM memberships m JOIN workspaces w ON w.id = m.workspace_id WHERE m.user_id = $1 AND NOT w.personal ORDER BY w.name")
        .bind(user_id)
        .fetch_all(&s.pool)
        .await?;
    let want = team.to_lowercase();
    if let Some((id, _)) = teams.iter().find(|(_, n)| n.to_lowercase() == want) {
        return Ok(*id);
    }
    let close: Vec<&(Uuid, String)> = teams.iter().filter(|(_, n)| n.to_lowercase().contains(&want)).collect();
    match close.as_slice() {
        [(id, _)] => Ok(*id),
        _ if teams.is_empty() => tool_err(format!("You're not in a team named \"{team}\". Leave team out to make the kit in your personal workspace.")),
        _ => tool_err(format!("No single team matches \"{team}\". Your teams: {}", teams.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

/// Asset ids the user may add to a kit: their own live image uploads. Returns the ones that aren't.
async fn foreign_assets(s: &AppState, user_id: Uuid, ids: &[String]) -> Result<Vec<String>, Fail> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ok: Vec<String> = sqlx::query_scalar("SELECT id FROM assets WHERE id = ANY($1) AND owner_id = $2 AND trashed_at IS NULL AND kind IN ('image','svg')")
        .bind(ids)
        .bind(user_id)
        .fetch_all(&s.pool)
        .await?;
    Ok(ids.iter().filter(|i| !ok.contains(i)).cloned().collect())
}

fn ids(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
}

/// Check the guidelines' total size before anything is saved, so a kit is never left half made.
fn guidelines_fit(a: &Value) -> Result<(), Fail> {
    let total: usize = a["guidelines"].as_array().into_iter().flatten().map(|x| x["body"].as_str().map_or(0, |b| b.chars().count())).sum();
    if total > MAX_GUIDELINES_TOTAL {
        return tool_err(format!("Guidelines can hold up to {MAX_GUIDELINES_TOTAL} characters in all; these have {total}. Shorten them or leave the least useful sections out."));
    }
    Ok(())
}

/// A kit as `kit_body` would store it, or the reasons it can't be.
fn checked(mut kit: Value) -> Result<Value, Fail> {
    // Fields an older kit never set take the schema's defaults.
    if let Some(m) = kit.as_object_mut() {
        m.retain(|_, v| !v.is_null());
    }
    hub::kit_body(false, false).safe_parse(Some(&kit)).map_err(|issues| {
        let lines: Vec<String> = issues
            .iter()
            .map(|i| {
                let path: Vec<String> = i["path"].as_array().into_iter().flatten().map(|p| p.as_str().map_or_else(|| p.to_string(), str::to_owned)).collect();
                format!("{} at {}", i["message"].as_str().unwrap_or("invalid"), path.join("."))
            })
            .collect();
        Fail::Tool(format!("The kit isn't valid: {}", lines.join("; ")))
    })
}

async fn write_guidelines(s: &AppState, u: &BearerUser, ws: Uuid, kit_id: &str, a: &Value) -> Result<(), Fail> {
    let Some(sections) = a.get("guidelines").and_then(Value::as_array) else { return Ok(()) };
    let w = hub::write_guidelines(s, kit_id, sections).await?.map_err(Fail::Tool)?;
    if w.changed {
        let mut meta = Map::new();
        meta.insert("from".into(), json!(w.before));
        meta.insert("to".into(), json!(w.after));
        audit(s, u, ws, "brand.kit_guidelines_updated", kit_id, meta).await?;
    }
    Ok(())
}

/// The kit as assistants see it (`get_brand_kit`), after a write.
async fn kit_reply(s: &AppState, u: &BearerUser, kit_id: &str, done: &str) -> R {
    let kits = hub::accessible_kits(s, u.id).await?;
    let Some(k) = kits.iter().find(|k| k["id"].as_str() == Some(kit_id)) else { return Ok(text(&json!({ "ok": true, "note": done }))) };
    let view = kit_views(s, u.id, &[k]).await?.pop().unwrap_or(Value::Null);
    Ok(text(&json!({ "ok": true, "done": done, "kit": view, "next": NEXT })))
}

fn duplicate(kits: &[Value], name: &str, except: Option<&str>) -> bool {
    let want = name.trim().to_lowercase();
    kits.iter().any(|k| k["name"].as_str().is_some_and(|n| n.trim().to_lowercase() == want) && k["id"].as_str() != except)
}

pub async fn create(s: &AppState, u: &BearerUser, a: &Value) -> R {
    need(u, "designs:write")?;
    let name = a["name"].as_str().unwrap_or("").trim().to_owned();
    let ws = target_workspace(s, u.id, opt_str(a, "team").as_deref()).await?;
    if hub::kit_role(s, u.id, ws).await? != Some("edit") {
        return tool_err("Only owners, admins and designers of that team can create brand kits. Leave team out to make it in your personal workspace.");
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM brand_kits WHERE workspace_id = $1").bind(ws).fetch_one(&s.pool).await?;
    if n >= i64::from(MAX_BRAND_KITS) {
        return tool_err(format!("That workspace already has {MAX_BRAND_KITS} brand kits, the most it can hold."));
    }
    let kits = hub::accessible_kits(s, u.id).await?;
    if duplicate(&kits, &name, None) {
        return tool_err(format!("There's already a brand kit named \"{name}\". Change it with update_brand_kit, or give this one another name."));
    }
    let images: Vec<String> = IMAGE_LISTS.iter().flat_map(|k| ids(a.get(*k))).collect();
    let bad = foreign_assets(s, u.id, &images).await?;
    if !bad.is_empty() {
        return tool_err(format!("These aren't image uploads of yours: {}. Upload the files first with upload_image or create_upload_link, and use the asset ids they return (list_uploads lists them).", bad.join(", ")));
    }
    guidelines_fit(a)?;
    let mut kit = Map::new();
    kit.insert("name".into(), json!(name));
    for (arg, field) in FIELDS {
        if let Some(v) = a.get(arg) {
            kit.insert(field.into(), v.clone());
        }
    }
    for key in IMAGE_LISTS {
        if let Some(v) = a.get(key) {
            kit.insert(key.into(), v.clone());
        }
    }
    let kit = checked(Value::Object(kit))?;
    let id = hub::save_kit(s, ws, u.id, &kit, None).await?;
    audit(s, u, ws, "brand.kit_created", &id, Map::new()).await?;
    write_guidelines(s, u, ws, &id, a).await?;
    kit_reply(s, u, &id, &format!("Created the brand kit \"{name}\". The user can see and change it in Brand Hub.")).await
}

pub async fn update(s: &AppState, u: &BearerUser, a: &Value) -> R {
    need(u, "designs:write")?;
    let kits = hub::accessible_kits(s, u.id).await?;
    let k = find_kit(&kits, a["brand_kit"].as_str().unwrap_or("")).map_err(Fail::Tool)?;
    if k["canEdit"] != true {
        return tool_err(format!("You can use the brand kit \"{}\" but not change it: only its team's owners, admins and designers can.", k["name"].as_str().unwrap_or("")));
    }
    let kit_id = k["id"].as_str().unwrap_or("").to_owned();
    let ws = k["workspaceId"].as_str().and_then(|w| Uuid::parse_str(w).ok()).ok_or_else(|| Fail::Other("brand kit without a workspace".into()))?;
    let mut next = Map::new();
    for key in ["name", "colors", "palettes", "fonts", "logos", "photos", "graphics", "voice", "textStyles"] {
        next.insert(key.into(), k.get(key).cloned().unwrap_or(Value::Null));
    }
    let mut touched = a.get("guidelines").is_some();
    if let Some(n) = opt_str(a, "new_name").map(|n| n.trim().to_owned()) {
        if duplicate(&kits, &n, Some(&kit_id)) {
            return tool_err(format!("There's already a brand kit named \"{n}\"."));
        }
        next.insert("name".into(), json!(n));
        touched = true;
    }
    for (arg, field) in FIELDS {
        let Some(v) = a.get(arg) else { continue };
        touched = true;
        if field == "fonts" {
            // Fonts merge by role: roles left out keep their font, and "" clears one.
            let mut fonts = next.get("fonts").and_then(Value::as_object).cloned().unwrap_or_default();
            for (role, family) in v.as_object().into_iter().flatten() {
                match family.as_str().map(str::trim) {
                    Some("") | None => fonts.remove(role),
                    Some(f) => fonts.insert(role.clone(), json!(f)),
                };
            }
            next.insert("fonts".into(), Value::Object(fonts));
        } else {
            next.insert(field.into(), v.clone());
        }
    }
    // Image lists: replaced when given, then added to, then pruned.
    let before: Vec<String> = IMAGE_LISTS.iter().flat_map(|key| ids(k.get(*key))).collect();
    for key in IMAGE_LISTS {
        let mut list = if a.get(key).is_some() { ids(a.get(key)) } else { ids(next.get(key)) };
        for id in ids(a.get(format!("add_{key}").as_str())) {
            if !list.contains(&id) {
                list.push(id);
            }
        }
        let remove = ids(a.get("remove_assets"));
        list.retain(|i| !remove.contains(i));
        touched |= a.get(key).is_some() || a.get(format!("add_{key}").as_str()).is_some() || a.get("remove_assets").is_some();
        next.insert(key.into(), json!(list));
    }
    if !touched {
        return tool_err("Nothing to change. Pass the fields to change, such as colors, fonts, add_logos or guidelines.");
    }
    let added: Vec<String> = IMAGE_LISTS.iter().flat_map(|key| ids(next.get(*key))).filter(|i| !before.contains(i)).collect();
    let bad = foreign_assets(s, u.id, &added).await?;
    if !bad.is_empty() {
        return tool_err(format!("These aren't image uploads of yours: {}. Upload the files first with upload_image or create_upload_link, and use the asset ids they return (list_uploads lists them).", bad.join(", ")));
    }
    guidelines_fit(a)?;
    let next = checked(Value::Object(next))?;
    // What changed, old and new, for the audit log, as the Brand Hub route records it.
    let mut changed = Map::new();
    for (key, v) in next.as_object().into_iter().flatten() {
        let old = k.get(key).cloned().unwrap_or(Value::Null);
        if &old != v {
            changed.insert(key.clone(), json!({ "from": old, "to": v }));
        }
    }
    hub::save_kit(s, ws, u.id, &next, Some(&kit_id)).await?;
    if !changed.is_empty() {
        let mut meta = Map::new();
        meta.insert("changed".into(), Value::Object(changed));
        audit(s, u, ws, "brand.kit_updated", &kit_id, meta).await?;
    }
    write_guidelines(s, u, ws, &kit_id, a).await?;
    let name = next["name"].as_str().unwrap_or("").to_owned();
    kit_reply(s, u, &kit_id, &format!("Updated the brand kit \"{name}\".")).await
}
