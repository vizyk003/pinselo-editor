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
    /// The first kit's text styles that has any, each with its font (`styles_view`).
    text_styles: Vec<Value>,
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
    type Json = Option<sqlx::types::Json<Value>>;
    type Row = (String, Json, Json, Json, Json);
    let kits: Vec<Row> = sqlx::query_as("SELECT name, colors, palettes, fonts, text_styles FROM brand_kits WHERE workspace_id = $1 ORDER BY is_default DESC, created_at")
        .bind(ws)
        .fetch_all(&s.pool)
        .await
        .map_err(|e| Fail::Other(e.to_string()))?;
    if kits.is_empty() {
        return Ok(None);
    }
    let mut r = Rules { kits: Vec::new(), fonts: Map::new(), colors: Vec::new(), text_styles: Vec::new(), enforce_colors: false, enforce_fonts: false };
    for (name, colors, palettes, fonts, styles) in &kits {
        if r.text_styles.is_empty() {
            r.text_styles = styles_view(&json!({ "fonts": fonts.as_ref().map(|f| f.0.clone()), "textStyles": styles.as_ref().map(|t| t.0.clone()) }));
        }
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
        if !self.text_styles.is_empty() {
            o["textStyles"] = json!(self.text_styles);
        }
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

// ---------------------------------------------------------------- text styles

/// A kit's text styles as assistants see them: each with the font its role has in the kit.
pub fn styles_view(kit: &Value) -> Vec<Value> {
    kit["textStyles"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|st| {
            let mut o = st.as_object().cloned().unwrap_or_default();
            if let Some(f) = st["role"].as_str().and_then(|r| kit["fonts"][r].as_str()).filter(|f| !f.is_empty()) {
                o.insert("font".into(), json!(f));
            }
            Value::Object(o)
        })
        .collect()
}

/// The kit a design's text is held to: the one it was made with (`meta.brandKit`, while this user can see it), else
/// the default kit of the team that governs it. `None` when there is neither, or the kit sets no fonts or styles.
pub struct StyleKit {
    pub name: String,
    fonts: Map<String, Value>,
    styles: Vec<Value>,
}

pub async fn style_kit(s: &AppState, user_id: Uuid, design_id: &str, doc: &Value) -> Result<Option<StyleKit>, Fail> {
    type Row = (String, sqlx::types::Json<Value>, sqlx::types::Json<Value>);
    let mut row: Option<Row> = None;
    if let Some(kit_id) = doc.pointer("/meta/brandKit/id").and_then(Value::as_str) {
        row = sqlx::query_as(
            "SELECT k.name, k.fonts, k.text_styles FROM brand_kits k JOIN memberships m ON m.workspace_id = k.workspace_id AND m.user_id = $2 WHERE k.id::text = $1",
        )
        .bind(kit_id)
        .bind(user_id)
        .fetch_optional(&s.pool)
        .await
        .map_err(|e| Fail::Other(e.to_string()))?;
    }
    if row.is_none()
        && let Some(ws) = governing_workspace(s, design_id).await?
    {
        row = sqlx::query_as("SELECT name, fonts, text_styles FROM brand_kits WHERE workspace_id = $1 ORDER BY is_default DESC, created_at LIMIT 1")
            .bind(ws)
            .fetch_optional(&s.pool)
            .await
            .map_err(|e| Fail::Other(e.to_string()))?;
    }
    let Some((name, fonts, styles)) = row else { return Ok(None) };
    let fonts: Map<String, Value> = fonts.0.as_object().cloned().unwrap_or_default().into_iter().filter(|(_, f)| f.as_str().is_some_and(|f| !f.trim().is_empty())).collect();
    let styles = styles.0.as_array().cloned().unwrap_or_default();
    if fonts.is_empty() && styles.is_empty() {
        return Ok(None);
    }
    Ok(Some(StyleKit { name, fonts, styles }))
}

/// The motion rules of the kit a design follows (the one it was made with, else its team's default kit), as defaults
/// for edit_design's animations and page transitions; an empty kit when there are none.
pub async fn motion_kit(s: &AppState, user_id: Uuid, design_id: &str, doc: &Value) -> Result<super::motion::Kit, Fail> {
    type Row = (String, sqlx::types::Json<Value>);
    let mut row: Option<Row> = None;
    if let Some(kit_id) = doc.pointer("/meta/brandKit/id").and_then(Value::as_str) {
        row = sqlx::query_as("SELECT k.name, k.motion FROM brand_kits k JOIN memberships m ON m.workspace_id = k.workspace_id AND m.user_id = $2 WHERE k.id::text = $1")
            .bind(kit_id)
            .bind(user_id)
            .fetch_optional(&s.pool)
            .await
            .map_err(|e| Fail::Other(e.to_string()))?;
    }
    if row.is_none()
        && let Some(ws) = governing_workspace(s, design_id).await?
    {
        row = sqlx::query_as("SELECT name, motion FROM brand_kits WHERE workspace_id = $1 ORDER BY is_default DESC, created_at LIMIT 1")
            .bind(ws)
            .fetch_optional(&s.pool)
            .await
            .map_err(|e| Fail::Other(e.to_string()))?;
    }
    Ok(match row {
        Some((name, m)) => super::motion::Kit::new(Some(name), m.0),
        None => super::motion::Kit::default(),
    })
}

/// The kit role a text element's semantic role stands for; `None` (a call to action, or no role) means "by its font".
fn kit_role(role: Option<&str>) -> Option<&'static str> {
    match role? {
        "headline" => Some("heading"),
        "subhead" => Some("subheading"),
        "body" | "legal" | "price" => Some("body"),
        "quote" => Some("quote"),
        "caption" => Some("caption"),
        _ => None,
    }
}

/// `"Találkozzunk Pasaréten"`, cut to a few words, to name a text in a message.
fn quote_text(t: &str) -> String {
    let one = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = one.chars().take(40).collect();
    if cut.len() < one.len() { format!("\"{cut}…\"") } else { format!("\"{cut}\"") }
}

fn num_text(n: f64) -> String {
    crate::js::num((n * 100.0).round() / 100.0)
}

/// How a text (its `get_design` view: font, weight, italic, case, letterSpacing, lineHeight, color, text) differs from a
/// style: one phrase per difference. Only what the style sets is compared, except that a style that doesn't ask for
/// italic or capitals means upright, mixed-case text.
fn differences(st: &Value, t: &Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(w) = st["weight"].as_f64() {
        let have = t.get("weight").and_then(Value::as_f64).unwrap_or(400.0);
        if (have - w).abs() >= 1.0 {
            out.push(format!("weight is {} (style: {})", num_text(have), num_text(w)));
        }
    }
    let italic = t.get("italic").and_then(Value::as_bool).unwrap_or(false);
    let want_italic = st["italic"].as_bool().unwrap_or(false);
    if italic != want_italic {
        out.push(if want_italic { "isn't italic (style: italic)".into() } else { "is italic (style: upright)".into() });
    }
    let case = t.get("case").and_then(Value::as_str);
    let text = t.get("text").and_then(Value::as_str).unwrap_or("");
    let typed_caps = text.chars().any(char::is_alphabetic) && !text.chars().any(char::is_lowercase);
    if st["uppercase"].as_bool().unwrap_or(false) {
        if case != Some("upper") && !typed_caps {
            out.push("isn't in capitals (style: uppercase, case \"upper\")".into());
        }
    } else if case == Some("upper") {
        out.push("is set in capitals (style: normal case)".into());
    }
    let want_ls = st["letterSpacing"].as_f64().unwrap_or(0.0);
    let have_ls = t.get("letterSpacing").and_then(Value::as_f64).unwrap_or(0.0);
    if (have_ls - want_ls).abs() > 15.0 {
        out.push(format!("letterSpacing is {} (style: {}; thousandths of an em)", num_text(have_ls), num_text(want_ls)));
    }
    if let Some(lh) = st["lineHeight"].as_f64() {
        let have = t.get("lineHeight").and_then(Value::as_f64).unwrap_or(1.3);
        if (have - lh).abs() > 0.1 {
            out.push(format!("lineHeight is {} (style: {})", num_text(have), num_text(lh)));
        }
    }
    let allowed: Vec<String> = st["colors"].as_array().into_iter().flatten().filter_map(Value::as_str).filter_map(norm).collect();
    if !allowed.is_empty()
        && let Some(c) = t.get("color").and_then(Value::as_str).and_then(norm)
        && !allowed.contains(&c)
    {
        out.push(format!("colour is {c} (style: {})", allowed.join(" or ")));
    }
    out
}

/// How many things a style asks for beyond the defaults.
fn traits(st: &Value) -> usize {
    let set = |k: &str| match &st[k] {
        Value::Bool(b) => *b,
        Value::Number(n) => k != "letterSpacing" || n.as_f64() != Some(0.0),
        Value::Array(a) => !a.is_empty(),
        _ => false,
    };
    ["weight", "italic", "uppercase", "letterSpacing", "lineHeight", "colors"].into_iter().filter(|k| set(k)).count()
}

impl StyleKit {
    fn font_of(&self, role: &str) -> Option<&str> {
        self.fonts.get(role).and_then(Value::as_str)
    }

    /// Problems with the given text elements (raw document elements by id), in `check`'s shape: `{ id, problem, message }`.
    pub fn check<'a>(&self, elements: impl IntoIterator<Item = (&'a str, &'a Value)>) -> Vec<Value> {
        let mut issues = Vec::new();
        for (id, el) in elements {
            if el["type"] != "text" {
                continue;
            }
            let t = super::view::text_out(&el["body"], 200);
            let font = t.get("font").and_then(Value::as_str).unwrap_or("");
            let name = quote_text(t.get("text").and_then(Value::as_str).unwrap_or(""));
            let role = kit_role(el["role"].as_str());
            if let Some(r) = role
                && let Some(want) = self.font_of(r)
                && !want.eq_ignore_ascii_case(font)
            {
                issues.push(json!({
                    "id": id,
                    "problem": "brand_font",
                    "message": format!("{name} ({}) is set in {font}; the kit \"{}\" uses {want} for {r} text.", el["role"].as_str().unwrap_or(""), self.name),
                }));
                continue;
            }
            let candidates: Vec<&Value> = self
                .styles
                .iter()
                .filter(|st| match role {
                    Some(r) => st["role"] == r,
                    None => st["role"].as_str().and_then(|r| self.font_of(r)).is_some_and(|f| f.eq_ignore_ascii_case(font)),
                })
                .collect();
            // Fewest differences; on a tie, the style whose own traits (capitals, italic, spacing…) the text shares most.
            let best = candidates
                .iter()
                .map(|st| (*st, differences(st, &t)))
                .min_by_key(|(st, d)| (d.len(), std::cmp::Reverse(traits(st).saturating_sub(d.len()))));
            if let Some((st, diff)) = best
                && !diff.is_empty()
            {
                let others = if candidates.len() > 1 { format!(" (the closest of its {} styles for this text)", candidates.len()) } else { String::new() };
                issues.push(json!({
                    "id": id,
                    "problem": "brand_style",
                    "message": format!("{name} doesn't follow the kit's \"{}\" text style{others}: {}.", st["name"].as_str().unwrap_or(""), diff.join("; ")),
                }));
            }
        }
        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(colors: bool, fonts: bool) -> Rules {
        let mut f = Map::new();
        f.insert("heading".into(), json!("Playfair Display"));
        f.insert("body".into(), json!("Inter"));
        Rules { kits: vec!["Acme".into()], fonts: f, colors: vec!["#0c7a80".into(), "#ffffff".into()], text_styles: vec![], enforce_colors: colors, enforce_fonts: fonts }
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

    fn kit() -> StyleKit {
        let fonts = json!({ "heading": "Canter", "caption": "Hanken Grotesk", "body": "Hanken Grotesk" }).as_object().cloned().unwrap();
        let styles = json!([
            { "name": "Heading", "role": "heading", "weight": 700, "colors": ["#943429", "#ffffff"] },
            { "name": "Caption", "role": "caption", "weight": 400, "italic": true },
            { "name": "Eyebrow", "role": "caption", "weight": 500, "uppercase": true, "letterSpacing": 140 },
        ]);
        StyleKit { name: "Pasaréti Ferencesek".into(), fonts, styles: styles.as_array().cloned().unwrap() }
    }

    fn text(role: &str, family: &str, text: &str, run: Value) -> Value {
        let paint = |h: &str| super::super::edit::solid(super::super::edit::hex(h));
        let mut d = json!({ "family": family, "weight": 400, "size": 20, "fill": paint("#404041"), "tracking": 0 });
        for (k, v) in run.as_object().cloned().unwrap_or_default() {
            d[k.as_str()] = if k == "fill" { paint(v.as_str().unwrap_or("")) } else { v };
        }
        json!({ "type": "text", "role": role, "body": { "defaultRun": d, "paragraphs": [{ "runs": [{ "text": text, "style": {} }] }] } })
    }

    #[test]
    fn text_styles_flag_the_closest_style() {
        let k = kit();
        // Rebi's eyebrow: typed in capitals, 600, letter spacing 2.
        let eyebrow = text("caption", "Hanken Grotesk", "PÁDUAI SZENT ANTAL PLÉBÁNIA", json!({ "weight": 600, "tracking": 2 }));
        let ok = text("caption", "Hanken Grotesk", "Fotó: P. Ferenc", json!({ "italic": true }));
        let heading = text("headline", "Canter", "Találkozzunk", json!({ "weight": 800 }));
        let wrong_font = text("headline", "Inter", "Hello", json!({ "weight": 700 }));
        let issues = k.check([("a", &eyebrow), ("b", &ok), ("c", &heading), ("d", &wrong_font)]);
        assert_eq!(issues.len(), 3, "{issues:#?}");
        let m = issues[0]["message"].as_str().unwrap();
        assert!(m.contains("\"Eyebrow\"") && m.contains("weight is 600 (style: 500)") && m.contains("letterSpacing is 2 (style: 140"), "{m}");
        assert!(!m.contains("capitals"), "typed capitals count as uppercase: {m}");
        assert_eq!(issues[1]["id"], "c");
        assert!(issues[1]["message"].as_str().unwrap().contains("weight is 800 (style: 700)"));
        assert_eq!(issues[2]["problem"], "brand_font");
    }

    #[test]
    fn text_styles_check_colours_and_skip_unstyled_text() {
        let k = kit();
        let red = text("headline", "Canter", "Cím", json!({ "weight": 700, "fill": "#FF0000" }));
        let issues = k.check([("a", &red)]);
        assert!(issues[0]["message"].as_str().unwrap().contains("colour is #ff0000 (style: #943429 or #ffffff)"), "{issues:#?}");
        // A call to action in a font no style uses, and a body text with no body style: nothing to check.
        let cta = text("cta", "Inter", "Tovább", json!({}));
        let body = text("body", "Hanken Grotesk", "Szöveg", json!({ "weight": 300 }));
        assert!(k.check([("b", &cta), ("c", &body)]).is_empty());
    }

    #[test]
    fn styles_view_adds_fonts() {
        let v = styles_view(&json!({ "fonts": { "caption": "Hanken Grotesk" }, "textStyles": [{ "name": "Eyebrow", "role": "caption" }, { "name": "Quote", "role": "quote" }] }));
        assert_eq!(v[0]["font"], "Hanken Grotesk");
        assert!(v[1].get("font").is_none());
    }
}
