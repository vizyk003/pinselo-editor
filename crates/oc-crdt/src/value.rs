// SPDX-License-Identifier: Apache-2.0
//! JSON ⇄ yrs value conversion with yjs semantics.
//!
//! yjs stores plain values as lib0 "any" (`ContentAny`); yrs reads and writes the same encoding as [`Any`].
//! JS has a single number type, so integral floats are read back as JSON integers (`1080`, never `1080.0`),
//! non-finite numbers become `null` (what `JSON.stringify` does) and `undefined` members are dropped.
//! Object keys are emitted in sorted order: yrs keeps map entries in a hash map, so the yjs insertion order
//! cannot be reproduced and a stable order is the next best thing (compare snapshots with [`canonical`]).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Number as JsonNumber, Value};
use yrs::any::Number;
use yrs::types::ToJson;
use yrs::{Any, Out, ReadTxn};

const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

/// A JSON value as a yrs [`Any`] (what `ymap.set(k, v)` stores in yjs).
pub fn json_to_any(v: &Value) -> Any {
    match v {
        Value::Null => Any::Null,
        Value::Bool(b) => Any::Bool(*b),
        Value::Number(n) => num_to_any(n),
        Value::String(s) => Any::String(Arc::from(s.as_str())),
        Value::Array(a) => Any::Array(a.iter().map(json_to_any).collect::<Vec<_>>().into()),
        Value::Object(m) => {
            let map: HashMap<String, Any> = m.iter().map(|(k, v)| (k.clone(), json_to_any(v))).collect();
            Any::Map(Arc::new(map))
        }
    }
}

fn num_to_any(n: &JsonNumber) -> Any {
    if let Some(i) = n.as_i64() {
        // Same lib0 encoding choices as a JS number: small integers as varints, the rest as floats.
        if (i as f64).abs() <= MAX_SAFE {
            return Any::Number(Number::Int(i));
        }
        return Any::Number(Number::Float(i as f64));
    }
    let f = n.as_f64().unwrap_or(0.0);
    if f.is_finite() && f.fract() == 0.0 && f.abs() <= MAX_SAFE {
        Any::Number(Number::Int(f as i64))
    } else {
        Any::Number(Number::Float(f))
    }
}

/// A number as JSON with JS semantics (integral → integer, non-finite → `null`).
pub fn f64_to_json(f: f64) -> Value {
    if !f.is_finite() {
        return Value::Null;
    }
    if f.fract() == 0.0 && f.abs() <= MAX_SAFE {
        return Value::from(f as i64);
    }
    JsonNumber::from_f64(f).map(Value::Number).unwrap_or(Value::Null)
}

/// A yrs [`Any`] as JSON (`JSON.parse(JSON.stringify(v))` of the value yjs would return).
pub fn any_to_json(a: &Any) -> Value {
    match a {
        Any::Null | Any::Undefined => Value::Null,
        Any::Bool(b) => Value::Bool(*b),
        Any::Number(Number::Int(i)) => f64_to_json(*i as f64),
        Any::Number(Number::Float(f)) => f64_to_json(*f),
        Any::String(s) => Value::String(s.to_string()),
        Any::Buffer(b) => Value::Array(b.iter().map(|x| Value::from(*x)).collect()),
        Any::Array(a) => Value::Array(a.iter().map(any_to_json).collect()),
        Any::Map(m) => {
            let mut keys: Vec<&String> = m.keys().filter(|k| !matches!(m[*k], Any::Undefined)).collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), any_to_json(&m[k]))).collect())
        }
    }
}

/// Any yrs output value as JSON. Shared types are flattened with their `toJSON` form.
pub fn out_to_json<T: ReadTxn>(txn: &T, o: &Out) -> Value {
    match o {
        Out::Any(a) => any_to_json(a),
        other => any_to_json(&other.to_json(txn)),
    }
}

/// `Out::Any(Undefined)`: a key explicitly set to `undefined` (dropped from JSON objects).
pub fn is_undefined(o: &Out) -> bool {
    matches!(o, Out::Any(Any::Undefined))
}

/// JS truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    oc_schema::tree::truthy(v)
}

/// JS truthiness of an optional JSON value (`undefined` is falsy).
pub fn truthy_opt(v: Option<&Value>) -> bool {
    v.is_some_and(truthy)
}

/// JS truthiness of a yrs value (shared types are objects, hence truthy).
pub fn truthy_out(o: Option<&Out>) -> bool {
    match o {
        None => false,
        Some(Out::Any(a)) => match a {
            Any::Null | Any::Undefined => false,
            Any::Bool(b) => *b,
            Any::Number(Number::Int(i)) => *i != 0,
            Any::Number(Number::Float(f)) => *f != 0.0 && !f.is_nan(),
            Any::String(s) => !s.is_empty(),
            _ => true,
        },
        Some(_) => true,
    }
}

/// A copy of `v` with every object's keys sorted (recursively): order-insensitive comparison of snapshots.
pub fn canonical(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), canonical(&m[k]))).collect())
        }
        Value::Number(n) => match n.as_f64() {
            Some(f) if !n.is_i64() && !n.is_u64() => f64_to_json(f),
            _ => v.clone(),
        },
        _ => v.clone(),
    }
}

/// `JSON.stringify` of the canonical form: equality of two JSON values regardless of key order.
pub fn canonical_string(v: &Value) -> String {
    serde_json::to_string(&canonical(v)).unwrap_or_default()
}

/// An object's entries as a JSON map (empty for non-objects).
pub fn obj(v: Option<&Value>) -> Map<String, Value> {
    match v {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    }
}

/// Text formatting attributes (`Attrs`) from a JSON object.
pub fn json_to_attrs(m: &Map<String, Value>) -> yrs::types::Attrs {
    m.iter().map(|(k, v)| (Arc::from(k.as_str()), json_to_any(v))).collect()
}

/// Text formatting attributes as a JSON object (sorted keys).
pub fn attrs_to_json(a: &yrs::types::Attrs) -> Map<String, Value> {
    let mut keys: Vec<&Arc<str>> = a.keys().collect();
    keys.sort();
    keys.into_iter().map(|k| (k.to_string(), any_to_json(&a[k]))).collect()
}
