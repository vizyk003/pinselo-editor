//! Port of services/api/src/routes/openapi.ts — the OpenAPI 3.1 description of the public API, served by v1 at
//! `/v1/openapi.json` (this module registers no routes).

use serde_json::{Value, json};

use crate::http::ratelimit::RouteLimit;
use crate::prelude::Router;
use crate::state::AppState;

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
}

fn ref_(n: &str) -> Value {
    json!({ "$ref": format!("#/components/schemas/{n}") })
}

fn json_(schema: Value, description: &str) -> Value {
    json!({ "description": description, "content": { "application/json": { "schema": schema } } })
}

fn id_param(name: &str) -> Value {
    json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" } })
}

/// `{ 200|201|202|204: …, ...errors }`.
fn responses(code: &str, ok: Value) -> Value {
    let mut m = serde_json::Map::new();
    m.insert(code.to_owned(), ok);
    m.insert("401".into(), json!({ "description": "Missing or invalid token" }));
    m.insert("403".into(), json!({ "description": "Token lacks the required scope" }));
    m.insert("404".into(), json!({ "description": "Not found" }));
    Value::Object(m)
}

/// `OPENAPI(origin)`.
pub fn openapi(origin: &str) -> Value {
    json!({
        "openapi": "3.1.0",
        "info": { "title": "Pinselo API", "version": "1.0.0", "description": "Create, fill and export designs. Authenticate with a personal access token from Settings → Developers. Webhook requests carry `OC-Signature: t=<unix>,v1=<hex HMAC-SHA256(secret, \"<t>.<body>\")>`." },
        "servers": [{ "url": origin }],
        "security": [{ "bearer": [] }],
        "components": {
            "securitySchemes": { "bearer": { "type": "http", "scheme": "bearer", "description": "oc_pat_… token" } },
            "schemas": {
                "Design": { "type": "object", "properties": { "id": { "type": "string" }, "title": { "type": "string" }, "type": { "type": "string" }, "width": { "type": "number" }, "height": { "type": "number" }, "pages": { "type": "integer" }, "createdAt": { "type": "string", "format": "date-time" }, "updatedAt": { "type": "string", "format": "date-time" }, "editUrl": { "type": "string" }, "thumbnailUrl": { "type": "string" } } },
                "Export": { "type": "object", "properties": { "id": { "type": "string" }, "status": { "enum": ["queued", "running", "done", "error", "cancelled"] }, "progress": { "type": "number" }, "filename": { "type": "string" }, "size": { "type": "integer" }, "downloadUrl": { "type": "string" }, "error": { "type": "string" } } },
                "PrintOptions": {
                    "type": "object", "description": "Professional print PDF (format pdf-print): bleed, printer marks, PDF/X and the output colour profile.", "required": ["bleedMm", "cropMarks"],
                    "properties": {
                        "bleedMm": { "type": "number", "minimum": 0, "maximum": 10 }, "cropMarks": { "type": "boolean" }, "registrationMarks": { "type": "boolean" }, "colorBars": { "type": "boolean" },
                        "slugMm": { "type": "number", "minimum": 0, "maximum": 20 }, "standard": { "enum": ["none", "pdfx4", "pdfx1a"] }, "flatten": { "type": "boolean" },
                        "profile": { "description": "A bundled profile id (pso-coated-v3, pso-uncoated-v3, iso-coated-v2, srgb) or { id } of an uploaded profile", "oneOf": [{ "type": "string" }, { "type": "object", "required": ["id"], "properties": { "id": { "type": "string", "format": "uuid" } } }] },
                        "intent": { "enum": ["relative", "perceptual"] },
                    },
                },
                "Webhook": { "type": "object", "properties": { "id": { "type": "string" }, "url": { "type": "string" }, "events": { "type": "array", "items": { "type": "string" } }, "active": { "type": "boolean" } } },
            },
        },
        "paths": {
            "/v1/me": { "get": { "summary": "The token’s user and scopes", "responses": responses("200", json_(json!({ "type": "object" }), "OK")) } },
            "/v1/designs": {
                "get": { "summary": "List designs (scope designs:read)", "parameters": [{ "name": "limit", "in": "query", "schema": { "type": "integer", "maximum": 100 } }, { "name": "before", "in": "query", "schema": { "type": "string", "format": "date-time" } }, { "name": "q", "in": "query", "schema": { "type": "string" } }], "responses": responses("200", json_(json!({ "type": "object", "properties": { "items": { "type": "array", "items": ref_("Design") }, "next": { "type": ["string", "null"] } } }), "OK")) },
                "post": { "summary": "Create a design (scope designs:write)", "requestBody": { "content": { "application/json": { "schema": { "type": "object", "properties": { "title": { "type": "string" }, "preset": { "type": "string", "description": "Size preset id, e.g. ig-post" }, "width": { "type": "number" }, "height": { "type": "number" }, "document": { "type": "object", "description": "A full design document (see GET …/document)" } } } } } }, "responses": responses("201", json_(ref_("Design"), "Created")) },
            },
            "/v1/designs/{id}": { "get": { "summary": "Get a design", "parameters": [id_param("id")], "responses": responses("200", json_(ref_("Design"), "OK")) } },
            "/v1/designs/{id}/document": { "get": { "summary": "The design document (JSON)", "parameters": [id_param("id")], "responses": responses("200", json_(json!({ "type": "object" }), "OK")) } },
            "/v1/designs/{id}/autofill": { "post": { "summary": "Copy a template and fill its data-bound fields (scope designs:write)", "description": "Keys of `data` are the field names set with Bulk create → Connect data. Text fields take text; image fields take an https URL or an asset id.", "parameters": [id_param("id")], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["data"], "properties": { "title": { "type": "string" }, "data": { "type": "object", "additionalProperties": { "type": "string" } } } } } } }, "responses": responses("201", json_(json!({ "type": "object", "properties": { "id": { "type": "string" }, "editUrl": { "type": "string" } } }), "Created")) } },
            "/v1/designs/{id}/exports": { "post": { "summary": "Start an export (scope exports)", "parameters": [id_param("id")], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["format"], "properties": { "format": { "enum": ["pdf", "pdf-print", "pptx", "mp4", "gif", "m4a"] }, "quality": { "enum": ["720p", "1080p", "4k"] }, "print": ref_("PrintOptions") } } } } }, "responses": responses("202", json_(ref_("Export"), "Accepted")) } },
            "/v1/exports/{job}": { "get": { "summary": "Export status", "parameters": [id_param("job")], "responses": responses("200", json_(ref_("Export"), "OK")) } },
            "/v1/exports/{job}/file": { "get": { "summary": "Download a finished export", "parameters": [id_param("job")], "responses": responses("200", json!({ "description": "The file" })) } },
            "/v1/assets": { "post": { "summary": "Upload an image, video or audio file (scope assets:write)", "requestBody": { "content": { "multipart/form-data": { "schema": { "type": "object", "properties": { "file": { "type": "string", "format": "binary" } } } } } }, "responses": responses("201", json_(json!({ "type": "object" }), "Created")) } },
            "/v1/webhooks": {
                "get": { "summary": "List webhooks (scope webhooks)", "responses": responses("200", json_(json!({ "type": "object", "properties": { "items": { "type": "array", "items": ref_("Webhook") } } }), "OK")) },
                "post": { "summary": "Subscribe a URL to events; the response includes the signing secret", "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["url", "events"], "properties": { "url": { "type": "string" }, "events": { "type": "array", "items": { "enum": ["design.created", "design.deleted", "export.completed", "form.response", "comment.created", "site.published", "ping"] } } } } } } }, "responses": responses("201", json_(ref_("Webhook"), "Created")) },
            },
            "/v1/webhooks/{id}": { "delete": { "summary": "Delete a webhook", "parameters": [id_param("id")], "responses": responses("204", json!({ "description": "Deleted" })) } },
        },
    })
}
