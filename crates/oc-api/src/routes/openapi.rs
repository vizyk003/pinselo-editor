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
    let design_detail = json!({
                    "allOf": [ref_("Design"), { "type": "object", "properties": {
                        "version": { "type": "integer", "description": "Content version: +1 for every saved change. Send it with a JPEG export to refuse a design edited since." },
                        "pageList": { "type": "array", "items": { "type": "object", "properties": { "index": { "type": "integer", "description": "1-based page number" }, "id": { "type": "string" }, "name": { "type": "string" }, "hidden": { "type": "boolean" } } } },
                    } }],
                });
    let jpeg_export = json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "exp_…" }, "status": { "enum": ["rendering", "done", "failed"] }, "designId": { "type": "string" }, "designVersion": { "type": "integer" },
                        "createdAt": { "type": "string", "format": "date-time" }, "statusUrl": { "type": "string" },
                        "warnings": { "type": "array", "items": { "type": "object", "properties": { "code": { "type": "string" }, "message": { "type": "string" } } } },
                        "files": { "type": "array", "description": "When done, in the order asked for. Public links (no sign-in) until expiresAt.", "items": { "type": "object", "properties": { "page": { "type": "integer" }, "url": { "type": "string" }, "width": { "type": "integer" }, "height": { "type": "integer" }, "bytes": { "type": "integer" }, "contentType": { "const": "image/jpeg" } } } },
                        "expiresAt": { "type": "string", "format": "date-time" },
                        "error": { "type": "object", "description": "When failed", "properties": { "code": { "enum": ["version_changed", "invalid_export", "render_failed", "unavailable"] }, "message": { "type": "string" }, "currentVersion": { "type": "integer" } } },
                    },
                });
    let exports_post = json!({ "post": {
                "summary": "Start an export (scope exports or designs:export)",
                "description": "PDF, PPTX, video and audio are queued (202, poll /v1/exports/{job}). `format: \"jpeg\"` renders up to 10 pages as Instagram-ready JPEGs with public 24-hour links; `version` refuses a design edited since (409 version_changed); `wait` holds the request up to 25 s; an `Idempotency-Key` header replays the same export for 24 hours.",
                "parameters": [id_param("id"), { "name": "Idempotency-Key", "in": "header", "schema": { "type": "string", "maxLength": 200 } }],
                "requestBody": { "required": true, "content": { "application/json": { "schema": { "oneOf": [
                    { "type": "object", "required": ["format"], "properties": { "format": { "enum": ["pdf", "pdf-print", "pptx", "mp4", "gif", "m4a"] }, "quality": { "enum": ["720p", "1080p", "4k"] }, "print": ref_("PrintOptions") } },
                    { "type": "object", "required": ["format"], "additionalProperties": false, "properties": { "format": { "enum": ["jpeg", "jpg"] }, "version": { "type": "integer" }, "width": { "type": "integer", "minimum": 320, "maximum": 1440, "default": 1080 }, "quality": { "type": "integer", "minimum": 60, "maximum": 100, "default": 90 }, "pages": { "type": "array", "maxItems": 10, "items": { "type": "integer", "minimum": 1 } }, "wait": { "type": "boolean" } } },
                ] } } } },
                "responses": {
                    "200": json_(ref_("JpegExport"), "JPEG export done"),
                    "202": json_(json!({ "oneOf": [ref_("Export"), ref_("JpegExport")] }), "Queued, or a JPEG export still rendering"),
                    "409": { "description": "version_changed: the design is no longer at `version` (`currentVersion` in the body)" },
                    "422": { "description": "invalid_export" },
                    "429": { "description": "rate_limited (200 JPEG exports per token per 24 hours)" },
                },
            } });
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
                "DesignDetail": design_detail,
                "JpegExport": jpeg_export,
                "Webhook": { "type": "object", "properties": { "id": { "type": "string" }, "url": { "type": "string" }, "events": { "type": "array", "items": { "type": "string" } }, "active": { "type": "boolean" } } },
            },
        },
        "paths": {
            "/v1/me": { "get": { "summary": "The token’s user and scopes", "responses": responses("200", json_(json!({ "type": "object" }), "OK")) } },
            "/v1/designs": {
                "get": { "summary": "List designs (scope designs:read)", "parameters": [{ "name": "limit", "in": "query", "schema": { "type": "integer", "maximum": 100 } }, { "name": "before", "in": "query", "schema": { "type": "string", "format": "date-time" } }, { "name": "q", "in": "query", "schema": { "type": "string" } }], "responses": responses("200", json_(json!({ "type": "object", "properties": { "items": { "type": "array", "items": ref_("Design") }, "next": { "type": ["string", "null"] } } }), "OK")) },
                "post": { "summary": "Create a design (scope designs:write)", "requestBody": { "content": { "application/json": { "schema": { "type": "object", "properties": { "title": { "type": "string" }, "preset": { "type": "string", "description": "Size preset id, e.g. ig-post" }, "width": { "type": "number" }, "height": { "type": "number" }, "document": { "type": "object", "description": "A full design document (see GET …/document)" } } } } } }, "responses": responses("201", json_(ref_("Design"), "Created")) },
            },
            "/v1/designs/{id}": { "get": { "summary": "Get a design with its content version and pages (scope designs:read)", "description": "`id` may also be the design's edit link (URL-encoded).", "parameters": [id_param("id")], "responses": responses("200", json_(ref_("DesignDetail"), "OK")) } },
            "/v1/designs/{id}/document": { "get": { "summary": "The design document (JSON)", "parameters": [id_param("id")], "responses": responses("200", json_(json!({ "type": "object" }), "OK")) } },
            "/v1/designs/{id}/autofill": { "post": { "summary": "Copy a template and fill its data-bound fields (scope designs:write)", "description": "Keys of `data` are the field names set with Bulk create → Connect data. Text fields take text; image fields take an https URL or an asset id.", "parameters": [id_param("id")], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["data"], "properties": { "title": { "type": "string" }, "data": { "type": "object", "additionalProperties": { "type": "string" } } } } } } }, "responses": responses("201", json_(json!({ "type": "object", "properties": { "id": { "type": "string" }, "editUrl": { "type": "string" } } }), "Created")) } },
            "/v1/designs/{id}/exports": exports_post,
            "/v1/exports/{job}": { "get": { "summary": "Export status (a queued job's uuid, or exp_… of a JPEG export)", "parameters": [id_param("job")], "responses": responses("200", json_(json!({ "oneOf": [ref_("Export"), ref_("JpegExport")] }), "OK")) } },
            "/v1/files/{export}/{page}.jpg": { "get": { "summary": "A JPEG export's file: public, signed link from `files[].url` (no token)", "security": [], "parameters": [id_param("export"), id_param("page")], "responses": { "200": { "description": "image/jpeg" }, "404": { "description": "Expired, altered, or its token was revoked" } } } },
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
