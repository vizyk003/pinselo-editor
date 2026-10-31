// SPDX-License-Identifier: Apache-2.0
//! Boundary conversions.
//!
//! * Rust → JS: `serde-wasm-bindgen` builds plain JS objects directly (no JSON text). Maps become objects and
//!   `None`/unit become `null`, matching the TS shapes (optional fields the TS omits are `skip_serializing_if`).
//! * JS → Rust: values go through `serde_json::Value` first, so typed arguments are read exactly like the JSON
//!   contract the goldens verify (integers vs floats, `null`, unknown fields kept in `extra`).

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use wasm_bindgen::prelude::*;

fn serializer() -> serde_wasm_bindgen::Serializer {
    serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true).serialize_missing_as_null(true)
}

pub fn err(msg: impl std::fmt::Display) -> JsError {
    JsError::new(&msg.to_string())
}

/// A JS `TypeError` (what the TS throws when it dereferences a missing element).
pub fn type_error(msg: &str) -> JsValue {
    js_sys::TypeError::new(msg).into()
}

pub fn to_js<T: Serialize + ?Sized>(v: &T) -> Result<JsValue, JsError> {
    v.serialize(&serializer()).map_err(err)
}

/// `to_js` for infallible shapes (our own types); `null` if serialisation failed.
pub fn js<T: Serialize + ?Sized>(v: &T) -> JsValue {
    to_js(v).unwrap_or(JsValue::NULL)
}

/// JS value → JSON value (`undefined` → `null`).
pub fn value(v: &JsValue) -> Result<Value, JsError> {
    if v.is_undefined() || v.is_null() {
        return Ok(Value::Null);
    }
    serde_wasm_bindgen::from_value(v.clone()).map_err(err)
}

/// Typed argument (named in the error message).
pub fn arg<T: DeserializeOwned>(name: &str, v: &JsValue) -> Result<T, JsError> {
    serde_json::from_value(value(v)?).map_err(|e| err(format!("{name}: {e}")))
}

/// Optional typed argument (`undefined`/`null` → `None`).
pub fn opt_arg<T: DeserializeOwned>(name: &str, v: &JsValue) -> Result<Option<T>, JsError> {
    if v.is_undefined() || v.is_null() { Ok(None) } else { arg(name, v).map(Some) }
}

/// Typed value from JSON.
pub fn typed<T: DeserializeOwned>(name: &str, v: Value) -> Result<T, JsError> {
    serde_json::from_value(v).map_err(|e| err(format!("{name}: {e}")))
}

/// An element from JSON; schema-invalid shapes become `Element::Unknown` (like the editor's document views).
pub fn element(v: Value) -> oc_schema::Element {
    match v {
        Value::Object(m) => serde_json::from_value(Value::Object(m.clone())).unwrap_or(oc_schema::Element::Unknown(m)),
        _ => oc_schema::Element::Unknown(serde_json::Map::new()),
    }
}

/// `element(value(v))` for an optional JS element (`undefined` → `None`).
pub fn opt_element(v: &JsValue) -> Result<Option<oc_schema::Element>, JsError> {
    Ok(match value(v)? {
        Value::Null => None,
        x => Some(element(x)),
    })
}
