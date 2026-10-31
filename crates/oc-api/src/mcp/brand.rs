//! Brand rules for assistants: the governing team's brand kits (fonts by role, colours) shown in `get_design`, and the
//! team's brand controls applied to `edit_design` the way the editor applies them to its pickers (`activeControls` in
//! apps/web/src/editor/brandStore.ts): for non-admins, with the colour or font control on, only kit colours or fonts;
//! an empty kit never locks anyone out.

use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::brandhub::{brand_controls, control_on, governing_workspace, is_admin_role, role_in};
use crate::state::AppState;

use super::tools::Fail;

pub struct Rules {
    kits: Vec<String>,
    /// heading/subheading/body/quote/caption → family (the first kit that sets one).
    fonts: Map<String, Value>,
    /// Lower-case #rrggbb, kit colours and palettes, in kit order.
    colors: Vec<String>,
    enforce_colors: bool,
    enforce_fonts: bool,
}

/// Lower-case #rrggbb (#abc expanded, alpha dropped); `None` for anything that isn't a hex colour.
fn norm(c: &str) -> Option<String> {
    let h = c.strip_prefix('#')?.to_ascii_lowercase();
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match h.len() {
        3 | 4 => Some(format!("#{}", h.chars().take(3).flat_map(|c| [c, c]).collect::<String>())),
        6 | 8 => Some(format!("#{}", &h[..6])),
        _ => None,
    }
}

/// The brand rules on a design for this user; `None` when no team with a brand kit governs it.
pub async fn rules(s: &AppState, user_id: Uuid, design_id: &str) -> Result<Option<Rules>, Fail> {
    let Some(ws) = governing_workspace(s, design_id).await? else { return Ok(None) };
    type Row = (String, Option<sqlx::types::Json<Value>>, Option<sqlx::types::Json<Value>>, Option<sqlx::types::Json<Value>>);
    let kits: Vec<Row> = sqlx::query_as("SELECT name, colors, palettes, fonts FROM brand_kits WHERE workspace_id = $1 ORDER BY is_default DESC, created_at")
        .bind(ws)
        .fetch_all(&s.pool)
        .await
        .map_err(|e| Fail::Other(e.to_string()))?;
    if kits.is_empty() {
        return Ok(None);
    }
    let mut r = Rules { kits: Vec::new(), fonts: Map::new(), colors: Vec::new(), enforce_colors: false, enforce_fonts: false };
    for (name, colors, palettes, fonts) in &kits {
        r.kits.push(name.clone());
        let palette_colors = palettes.iter().flat_map(|p| p.0.as_array().cloned().unwrap_or_default()).flat_map(|p| p["colors"].as_array().cloned().unwrap_or_default());
        for c in colors.iter().flat_map(|c| c.0.as_array().cloned().unwrap_or_default()).chain(palette_colors) {
            if let Some(h) = c["hex"].as_str().and_then(norm)
                && !r.colors.contains(&h)
            {
                r.colors.push(h);
            }
        }
        for (role, family) in fonts.as_ref().and_then(|f| f.0.as_object().cloned()).unwrap_or_default() {
            if family.as_str().is_some_and(|f| !f.is_empty()) && !r.fonts.contains_key(&role) {
                r.fonts.insert(role, family);
            }
        }
    }
    let admin = is_admin_role(role_in(s, user_id, Some(ws)).await?.as_deref());
    let controls = brand_controls(s, Some(ws)).await?;
    r.enforce_colors = !admin && control_on(&controls, "colors") && !r.colors.is_empty();
    r.enforce_fonts = !admin && control_on(&controls, "fonts") && !r.fonts.is_empty();
    Ok(Some(r))
}

impl Rules {
    fn families(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for f in self.fonts.values().filter_map(Value::as_str) {
            if !v.contains(&f) {
                v.push(f);
            }
        }
        v
    }

    /// What `get_design` shows as `brand`.
    pub fn view(&self) -> Value {
        let mut o = json!({ "kits": self.kits, "fonts": self.fonts, "colors": self.colors.iter().take(40).collect::<Vec<_>>() });
        let mut enforced = Vec::new();
        if self.enforce_colors {
            enforced.push("colors");
        }
        if self.enforce_fonts {
            enforced.push("fonts");
        }
        if !enforced.is_empty() {
            o["enforced"] = json!(enforced);
            o["note"] = json!("The team's brand controls are on: edit_design only accepts these colours and fonts.");
        }
        o
    }

    /// The first operation that breaks an enforced control, as an error message.
    pub fn check(&self, ops: &[Value]) -> Option<String> {
        if !self.enforce_colors && !self.enforce_fonts {
            return None;
        }
        for (i, op) in ops.iter().enumerate() {
            let mut bad = None;
            self.walk(op, None, &mut bad);
            if let Some(m) = bad {
                return Some(format!("Operation {} ({}): {m}", i + 1, op["op"].as_str().unwrap_or("")));
            }
        }
        None
    }

    fn walk(&self, v: &Value, key: Option<&str>, bad: &mut Option<String>) {
        if bad.is_some() {
            return;
        }
        match v {
            Value::String(t) => match key {
                Some("color" | "fill" | "background" | "stroke" | "gradient") if self.enforce_colors => {
                    if let Some(h) = norm(t)
                        && !self.colors.contains(&h)
                    {
                        let list = self.colors.iter().take(24).cloned().collect::<Vec<_>>().join(", ");
                        *bad = Some(format!("{t} isn't one of the team's brand colours (brand controls are on). Use one of: {list}."));
                    }
                }
                Some("font") if self.enforce_fonts && !self.families().iter().any(|f| f.eq_ignore_ascii_case(t)) => {
                    *bad = Some(format!("{t} isn't one of the team's brand fonts (brand controls are on). Use one of: {}.", self.families().join(", ")));
                }
                _ => {}
            },
            Value::Array(a) => a.iter().for_each(|x| self.walk(x, key, bad)),
            Value::Object(m) => m.iter().for_each(|(k, x)| self.walk(x, Some(k.as_str()), bad)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(colors: bool, fonts: bool) -> Rules {
        let mut f = Map::new();
        f.insert("heading".into(), json!("Playfair Display"));
        f.insert("body".into(), json!("Inter"));
        Rules { kits: vec!["Acme".into()], fonts: f, colors: vec!["#0c7a80".into(), "#ffffff".into()], enforce_colors: colors, enforce_fonts: fonts }
    }

    #[test]
    fn normalises_hex() {
        assert_eq!(norm("#ABC").as_deref(), Some("#aabbcc"));
        assert_eq!(norm("#0C7A80ff").as_deref(), Some("#0c7a80"));
        assert_eq!(norm("red"), None);
    }

    #[test]
    fn checks_only_enforced_controls() {
        let ops = vec![
            json!({ "op": "add_text", "text": "Hi", "color": "#0C7A80", "font": "inter" }),
            json!({ "op": "add_shape", "fill": { "gradient": ["#ffffff", "#ff0000"] }, "stroke": { "color": "#fff" } }),
        ];
        assert_eq!(rules(false, false).check(&ops), None);
        assert_eq!(rules(false, true).check(&ops), None);
        let m = rules(true, false).check(&ops).unwrap();
        assert!(m.starts_with("Operation 2 (add_shape): #ff0000 isn't one of the team's brand colours"), "{m}");
        let m = rules(false, true).check(&[json!({ "op": "update", "id": "x", "font": "Comic Sans MS" })]).unwrap();
        assert!(m.contains("Use one of: Playfair Display, Inter."), "{m}");
    }
}
