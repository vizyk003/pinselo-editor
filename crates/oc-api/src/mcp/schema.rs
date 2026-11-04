//! The tools' input schemas (zod in services/api/src/mcp/tools.ts and edit.ts), as the MCP SDK applies them before a
//! tool runs: arguments are parsed (unknown keys dropped) and a failure becomes the tool error
//! `MCP error -32602: Input validation error: Invalid arguments for tool <name>: <issues>`. The JSON Schemas that
//! `tools/list` announces are the SDK's rendering of the same zod schemas, kept verbatim in `tools.json`.

use serde_json::{Value, json};

use crate::zod::{self as z, Schema};

const COLOR_JS: &str = "^#([0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$";
const COLOR_MESSAGE: &str = "a hex colour like #1f2328 or #1f232880";

pub const OPS: [&str; 21] = [
    "add_text", "add_shape", "add_line", "add_image", "add_table", "add_chart", "add_qr", "group", "ungroup", "update", "delete", "reorder", "arrange", "align",
    "distribute", "add_page", "update_page", "delete_page", "move_page", "set_title", "replace_text",
];

fn color() -> Schema {
    z::str().regex(COLOR_JS, COLOR_JS)
}
fn fill() -> Schema {
    z::union(vec![color(), z::obj([("gradient", z::array(color()).min(2.0).max(8.0)), ("angle", z::num().optional()), ("radial", z::bool().optional())])])
}
fn r#ref() -> Schema {
    z::str().max(40.0).optional()
}
fn target() -> Schema {
    z::str()
}
fn page_ref() -> Schema {
    z::union(vec![z::str(), z::num().int().min(1.0)])
}
/// `SemanticRole` values (oc-schema): what an element is for, so assistants and resize keep the hierarchy.
pub const ROLES: [&str; 12] = ["headline", "subhead", "body", "quote", "caption", "cta", "logo", "hero-image", "background", "decor", "price", "legal"];

fn placement() -> Vec<(&'static str, Schema)> {
    vec![
        ("page", page_ref().optional()),
        ("parent", target().optional()),
        ("x", z::num()),
        ("y", z::num()),
        ("rotation", z::num().optional()),
        ("opacity", z::num().min(0.0).max(1.0).optional()),
        ("name", z::str().max(80.0).optional()),
        ("role", z::enumeration(&ROLES).optional()),
    ]
}
fn font() -> Schema {
    z::str().max(80.0).optional()
}
fn size() -> Schema {
    z::num().min(1.0).max(2000.0).optional()
}
fn text_style() -> Vec<(&'static str, Schema)> {
    vec![
        ("font", font()),
        ("size", size()),
        ("weight", z::num().int().min(100.0).max(900.0).optional()),
        ("italic", z::bool().optional()),
        ("color", color().optional()),
        ("align", z::enumeration(&["left", "center", "right", "justify"]).optional()),
        ("lineHeight", z::num().min(0.5).max(4.0).optional()),
        ("letterSpacing", z::num().min(-500.0).max(2000.0).optional()),
        ("case", z::enumeration(&["none", "upper", "lower", "title"]).optional()),
    ]
}
fn stroke_in() -> Schema {
    z::obj([("color", color()), ("width", z::num().min(0.0).max(500.0)), ("dash", z::array(z::num().min(0.0)).max(8.0).optional())])
}
fn marker() -> Schema {
    z::enumeration(&["none", "arrow", "triangle", "stealth", "triangle-outline", "circle", "circle-outline", "square", "square-outline", "diamond", "diamond-outline", "bar"])
}
fn image_source() -> Schema {
    z::union(vec![
        z::obj([("url", z::str().url())]),
        z::obj([("asset", z::str())]),
        z::obj([("brand", z::obj([("kit", z::str().max(80.0)), ("name", z::str().max(255.0))]))]),
        z::obj([("stock", z::obj([("provider", z::str()), ("id", z::str()), ("type", z::enumeration(&["image", "gif", "sticker"]).optional())]))]),
    ])
}
fn positive() -> Schema {
    z::num().positive()
}

fn op_schema(op: &str) -> Schema {
    let lit = ("op", z::literal(json!(op)));
    let mut f: Vec<(&str, Schema)> = vec![lit];
    match op {
        "add_text" => {
            f.extend([("ref", r#ref()), ("text", z::str().max(20000.0))]);
            f.extend(placement());
            f.extend([("w", positive()), ("h", positive().optional())]);
            f.extend(text_style());
            f.push(("list", z::enumeration(&["bullet", "number"]).optional()));
        }
        "add_shape" => {
            f.extend([("ref", r#ref()), ("shape", z::str())]);
            f.extend(placement());
            f.extend([("w", positive()), ("h", positive()), ("fill", fill().optional()), ("stroke", stroke_in().optional()), ("radius", z::num().min(0.0).optional())]);
        }
        "add_line" => f.extend([
            ("ref", r#ref()),
            ("page", page_ref().optional()),
            ("parent", target().optional()),
            ("x1", z::num()),
            ("y1", z::num()),
            ("x2", z::num()),
            ("y2", z::num()),
            ("color", color().optional()),
            ("width", z::num().min(0.5).max(200.0).optional()),
            ("dash", z::array(z::num().min(0.0)).max(8.0).optional()),
            ("start", marker().optional()),
            ("end", marker().optional()),
            ("name", z::str().max(80.0).optional()),
        ]),
        "add_image" => {
            f.extend([("ref", r#ref()), ("source", image_source())]);
            f.extend(placement());
            f.extend([
                ("w", positive()),
                ("h", positive()),
                ("fit", z::enumeration(&["cover", "contain"]).optional()),
                ("radius", z::num().min(0.0).optional()),
                ("alt", z::str().max(500.0).optional()),
            ]);
        }
        "add_table" => {
            f.extend([("ref", r#ref()), ("data", z::array(z::array(z::str().max(2000.0)).max(30.0)).min(1.0).max(100.0))]);
            f.extend(placement());
            f.extend([
                ("w", positive()),
                ("h", positive().optional()),
                ("font", font()),
                ("size", size()),
                ("color", color().optional()),
                ("headerFill", color().optional()),
                ("headerColor", color().optional()),
            ]);
        }
        "add_chart" => {
            f.extend([
                ("ref", r#ref()),
                ("chart", z::str()),
                ("labels", z::array(z::str().max(80.0)).max(200.0)),
                ("series", z::array(z::obj([("name", z::str().max(80.0)), ("values", z::array(z::num()).max(200.0)), ("color", color().optional())])).min(1.0).max(12.0)),
            ]);
            f.extend(placement());
            f.extend([
                ("w", positive()),
                ("h", positive()),
                ("showLegend", z::bool().optional()),
                ("showValues", z::bool().optional()),
                ("font", font()),
                ("textColor", color().optional()),
            ]);
        }
        "add_qr" => {
            f.extend([("ref", r#ref()), ("data", z::str().min(1.0).max(2000.0))]);
            f.extend(placement());
            f.extend([("size", positive()), ("color", color().optional()), ("background", color().optional())]);
        }
        "group" => f.extend([("ref", r#ref()), ("ids", z::array(target()).min(2.0)), ("name", z::str().max(80.0).optional())]),
        "ungroup" => f.push(("id", target())),
        "update" => {
            f.extend([
                ("id", target()),
                ("x", z::num().optional()),
                ("y", z::num().optional()),
                ("w", positive().optional()),
                ("h", positive().optional()),
                ("rotation", z::num().optional()),
                ("opacity", z::num().min(0.0).max(1.0).optional()),
                ("name", z::str().max(80.0).optional()),
                ("role", z::enumeration(&ROLES).optional()),
                ("hidden", z::bool().optional()),
                ("text", z::str().max(20000.0).optional()),
            ]);
            f.extend(text_style());
            f.extend([
                ("fill", fill().optional()),
                ("stroke", stroke_in().nullable().optional()),
                ("radius", z::num().min(0.0).optional()),
                ("image", image_source().optional()),
                ("alt", z::str().max(500.0).optional()),
            ]);
        }
        "delete" => f.push(("ids", z::array(target()).min(1.0))),
        "reorder" => f.extend([("id", target()), ("to", z::enumeration(&["front", "back", "forward", "backward"]))]),
        "arrange" => f.extend([
            ("ids", z::array(target()).min(1.0)),
            ("direction", z::enumeration(&["row", "column", "grid"])),
            ("gap", z::num().min(0.0).optional()),
            ("columns", z::num().int().min(1.0).optional()),
            ("within", z::enumeration(&["page", "items"]).optional()),
            ("padding", z::num().min(0.0).optional()),
            ("x", z::num().optional()),
            ("y", z::num().optional()),
            ("width", positive().optional()),
            ("height", positive().optional()),
            ("align", z::enumeration(&["start", "center", "end"]).optional()),
            ("justify", z::enumeration(&["start", "center", "end", "space-between"]).optional()),
            ("stretch", z::bool().optional()),
        ]),
        "align" => f.extend([
            ("ids", z::array(target()).min(1.0)),
            ("to", z::enumeration(&["left", "center", "right", "top", "middle", "bottom"])),
            ("relativeTo", z::enumeration(&["page", "selection"]).optional()),
        ]),
        "distribute" => f.extend([("ids", z::array(target()).min(3.0)), ("axis", z::enumeration(&["horizontal", "vertical"]))]),
        "add_page" => f.extend([
            ("ref", r#ref()),
            ("after", page_ref().optional()),
            ("duplicate", page_ref().optional()),
            ("title", z::str().max(120.0).optional()),
            ("background", fill().optional()),
        ]),
        "update_page" => f.extend([("page", page_ref()), ("title", z::str().max(120.0).optional()), ("background", fill().optional()), ("notes", z::str().max(5000.0).optional())]),
        "delete_page" => f.push(("page", page_ref())),
        "move_page" => f.extend([("page", page_ref()), ("after", page_ref().nullable())]),
        "replace_text" => f.extend([
            ("find", z::str().min(1.0).max(500.0)),
            ("replace", z::str().max(2000.0)),
            ("match_case", z::bool().optional()),
            ("whole_word", z::bool().optional()),
            ("page", page_ref().optional()),
            ("ids", z::array(target()).min(1.0).optional()),
        ]),
        _ => f.push(("title", z::str().min(1.0).max(200.0))),
    }
    z::obj_vec(f)
}

fn prefixed(issues: Vec<Value>, prefix: &[Value]) -> Vec<Value> {
    issues
        .into_iter()
        .map(|mut i| {
            let mut p = prefix.to_vec();
            p.extend(i["path"].as_array().cloned().unwrap_or_default());
            i["path"] = Value::Array(p);
            i
        })
        .collect()
}

/// `z.array(EditOp).min(1).max(200)` (EditOp: a discriminated union on `op`).
fn operations(input: Option<&Value>, issues: &mut Vec<Value>) -> Option<Value> {
    let Some(Value::Array(items)) = input else {
        let received = match input {
            None => "undefined",
            Some(Value::Null) => "null",
            Some(Value::Object(_)) => "object",
            Some(Value::String(_)) => "string",
            Some(Value::Number(_)) => "number",
            Some(Value::Bool(_)) => "boolean",
            Some(Value::Array(_)) => "array",
        };
        issues.push(json!({ "expected": "array", "code": "invalid_type", "path": ["operations"], "message": format!("Invalid input: expected array, received {received}") }));
        return None;
    };
    // zod >= 4.6.6: more than 200 operations fail the max check first, and no operation is parsed.
    if items.len() > 200 {
        issues.push(json!({ "origin": "array", "code": "too_big", "maximum": 200, "inclusive": true, "path": ["operations"], "message": "Too big: expected array to have <=200 items" }));
        return None;
    }
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let prefix = [json!("operations"), json!(i)];
        let Some(obj) = item.as_object() else {
            let received = if item.is_null() { "null" } else if item.is_array() { "array" } else if item.is_string() { "string" } else if item.is_number() { "number" } else { "boolean" };
            issues.push(json!({ "expected": "object", "code": "invalid_type", "path": prefix, "message": format!("Invalid input: expected object, received {received}") }));
            continue;
        };
        match obj.get("op").and_then(Value::as_str).filter(|o| OPS.contains(o)) {
            None => {
                let expected = OPS.iter().map(|o| format!("'{o}'")).collect::<Vec<_>>().join(" | ");
                issues.push(json!({ "code": "invalid_union", "path": ["operations", i, "op"], "message": format!("Invalid discriminator value. Expected {expected}") }));
            }
            Some(op) => match op_schema(op).safe_parse(Some(item)) {
                Ok(v) => out.push(v),
                Err(e) => issues.extend(prefixed(e, &prefix)),
            },
        }
    }
    if items.is_empty() {
        issues.push(json!({ "origin": "array", "code": "too_small", "minimum": 1, "inclusive": true, "path": ["operations"], "message": "Too small: expected array to have >=1 items" }));
    }
    Some(Value::Array(out))
}

fn tool_schema(name: &str) -> Option<Schema> {
    let page = || z::union(vec![z::str(), z::num().int().min(1.0)]).optional();
    Some(match name {
        "get_design_guide" | "get_brand_kit" => z::obj([]),
        "list_designs" => z::obj([("query", z::str().max(200.0).optional()), ("limit", z::num().int().min(1.0).max(50.0).optional())]),
        "list_size_presets" => z::obj([("query", z::str().max(80.0).optional())]),
        "search_templates" => z::obj([("query", z::str().max(200.0).optional()), ("limit", z::num().int().min(1.0).max(40.0).optional())]),
        "search_media" => z::obj([
            ("query", z::str().min(1.0).max(100.0)),
            ("type", z::enumeration(&["image", "gif", "sticker"]).optional()),
            ("page", z::num().int().min(1.0).max(20.0).optional()),
        ]),
        "list_uploads" => z::obj([("query", z::str().max(100.0).optional()), ("limit", z::num().int().min(1.0).max(100.0).optional())]),
        "upload_image" => z::obj([
            ("filename", z::str().min(1.0).max(200.0).optional()),
            ("data_base64", z::str().min(1.0).optional()),
            // ChatGPT fills `file` in for a tool that declares it under `_meta["openai/fileParams"]`: an object with the
            // attachment's download_url; a bare string is what a model passes when it has only an id.
            (
                "file",
                z::union(vec![
                    z::str().max(4096.0),
                    z::obj([("download_url", z::str().optional()), ("file_id", z::str().optional()), ("mime_type", z::str().optional()), ("file_name", z::str().max(200.0).optional())]),
                ])
                .optional(),
            ),
        ]),
        "create_upload_link" => z::obj([("filename", z::str().max(200.0).optional()), ("purpose", z::str().max(200.0).optional())]),
        "get_upload_link" => z::obj([("id", z::str().uuid())]),
        "search_fonts" => z::obj([
            ("query", z::str().max(80.0).optional()),
            ("category", z::enumeration(&["sans", "serif", "display", "handwriting", "mono"]).optional()),
            ("limit", z::num().int().min(1.0).max(100.0).optional()),
        ]),
        "create_design" => z::obj([
            ("title", z::str().min(1.0).max(200.0)),
            ("preset", z::str().optional()),
            ("width", z::num().min(16.0).max(20000.0).optional()),
            ("height", z::num().min(16.0).max(20000.0).optional()),
            ("unit", z::enumeration(&["px", "mm", "cm", "in", "pt"]).optional()),
            ("template_id", z::str().optional()),
            ("brand_kit", z::str().max(80.0).optional()),
        ]),
        "get_design" => z::obj([("design_id", z::str()), ("page", page())]),
        "screenshot_page" => z::obj([("design_id", z::str()), ("page", page()), ("width", z::num().int().min(200.0).max(2000.0).optional())]),
        "export_design" => z::obj([
            ("design_id", z::str()),
            ("format", z::enumeration(&["png", "jpg", "pdf", "pdf-print", "pptx", "svg", "mp4", "gif"])),
            ("pages", z::array(z::num().int().min(1.0)).optional()),
            ("scale", z::num().min(0.25).max(4.0).optional()),
        ]),
        "undo_ai_changes" => z::obj([("design_id", z::str())]),
        "bulk_create" => z::obj([
            ("design_id", z::str()),
            ("rows", z::array(z::record(z::str().max(10000.0))).min(1.0).max(50.0)),
            ("titles", z::array(z::str().min(1.0).max(200.0)).max(50.0).optional()),
        ]),
        "copy_design" => z::obj([
            ("design_id", z::str()),
            ("title", z::str().min(1.0).max(200.0).optional()),
            ("preset", z::str().optional()),
            ("width", z::num().min(16.0).max(20000.0).optional()),
            ("height", z::num().min(16.0).max(20000.0).optional()),
            ("unit", z::enumeration(&["px", "mm", "cm", "in", "pt"]).optional()),
            ("scale_content", z::bool().optional()),
        ]),
        _ => return None,
    })
}

pub const TOOLS: [&str; 19] = [
    "get_design_guide", "list_designs", "list_size_presets", "search_templates", "search_media", "list_uploads", "upload_image", "create_upload_link", "get_upload_link",
    "get_brand_kit", "search_fonts", "create_design", "copy_design", "bulk_create", "get_design", "edit_design", "screenshot_page", "export_design", "undo_ai_changes",
];

/// `getDotPath(path)` of the SDK.
fn dot_path(path: &[Value]) -> String {
    let mut out = String::new();
    for (i, seg) in path.iter().enumerate() {
        match seg {
            Value::Number(n) if i > 0 => out.push_str(&format!("[{n}]")),
            Value::String(s) if i > 0 => {
                out.push('.');
                out.push_str(s);
            }
            Value::String(s) => out.push_str(s),
            other => out.push_str(&other.to_string()),
        }
    }
    out
}

/// `getParseErrorMessage(error)`: one line per issue, `<message> at <path>`.
fn issues_message(issues: &[Value]) -> String {
    issues
        .iter()
        .map(|i| {
            let mut m = i["message"].as_str().unwrap_or("").to_owned();
            if i["code"] == "invalid_format" && i["pattern"] == format!("/{COLOR_JS}/") {
                m = COLOR_MESSAGE.to_owned();
            }
            let path = i["path"].as_array().cloned().unwrap_or_default();
            if path.is_empty() { m } else { format!("{m} at {}", dot_path(&path)) }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Validate a tool's arguments (`args ?? {}`): the parsed arguments, or the SDK's error text.
pub fn validate(name: &str, args: Option<&Value>) -> Result<Value, String> {
    let empty = json!({});
    let input = args.unwrap_or(&empty);
    let fail = |issues: &[Value]| format!("MCP error -32602: Input validation error: Invalid arguments for tool {name}: {}", issues_message(issues));
    if name == "edit_design" {
        let Some(obj) = input.as_object() else {
            return Err(fail(&[json!({ "code": "invalid_type", "path": [], "message": "Invalid input: expected object, received array" })]));
        };
        let mut issues = Vec::new();
        let id = match z::obj([("design_id", z::str())]).safe_parse(Some(input)) {
            Ok(v) => v["design_id"].clone(),
            Err(e) => {
                issues.extend(e);
                Value::Null
            }
        };
        let ops = operations(obj.get("operations"), &mut issues);
        if !issues.is_empty() {
            return Err(fail(&issues));
        }
        return Ok(json!({ "design_id": id, "operations": ops }));
    }
    let schema = tool_schema(name).ok_or_else(|| format!("MCP error -32602: Tool {name} not found"))?;
    schema.safe_parse(Some(input)).map_err(|e| fail(&e))
}

/// A registered tool name.
pub fn is_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}
