//! Rows as node-postgres hands them to JavaScript, and `JSON.stringify(value, null, 2)` of them — what the
//! account export writes into its JSON files.
//!
//! node-postgres types: `uuid`/`text`/`inet`/`int8`/`numeric` → string, `int2`/`int4`/`float4`/`float8` → number,
//! `bool` → boolean, `timestamptz`/`timestamp` → `Date` (serialized as its ISO string, millisecond
//! precision), `json`/`jsonb` → parsed value, arrays of those → arrays, SQL NULL → `null`. Column order is
//! the order of the SELECT list.

use chrono::{DateTime, NaiveDateTime, Utc};
use oc_js::JsValue;
use sqlx::postgres::PgRow;
use sqlx::{Column, Row, TypeInfo, ValueRef};

fn date(t: DateTime<Utc>) -> JsValue {
    JsValue::String(oc_js::iso_string(&t))
}

fn json(v: &serde_json::Value) -> JsValue {
    JsValue::from(v)
}

/// One column of a row as node-postgres would return it.
pub fn column(row: &PgRow, i: usize) -> Result<JsValue, sqlx::Error> {
    if row.try_get_raw(i)?.is_null() {
        return Ok(JsValue::Null);
    }
    let ty = row.column(i).type_info().name().to_ascii_uppercase();
    Ok(match ty.as_str() {
        "UUID" => JsValue::String(row.try_get::<uuid::Uuid, _>(i)?.to_string()),
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CHAR" => JsValue::String(row.try_get::<String, _>(i)?),
        "INET" | "CIDR" => JsValue::String(row.try_get::<String, _>(i).unwrap_or_default()),
        "INT2" => JsValue::Number(f64::from(row.try_get::<i16, _>(i)?)),
        "INT4" => JsValue::Number(f64::from(row.try_get::<i32, _>(i)?)),
        "INT8" => JsValue::String(row.try_get::<i64, _>(i)?.to_string()),
        // Postgres prints a real with the shortest round-trip digits; node-postgres parses that text.
        "FLOAT4" => JsValue::Number(row.try_get::<f32, _>(i)?.to_string().parse().unwrap_or(f64::NAN)),
        "FLOAT8" => JsValue::Number(row.try_get::<f64, _>(i)?),
        "BOOL" => JsValue::Bool(row.try_get::<bool, _>(i)?),
        "TIMESTAMPTZ" => date(row.try_get::<DateTime<Utc>, _>(i)?),
        // node-postgres reads a timestamp without time zone as local time; the server runs in UTC.
        "TIMESTAMP" => date(row.try_get::<NaiveDateTime, _>(i)?.and_utc()),
        "JSON" | "JSONB" => json(&row.try_get::<serde_json::Value, _>(i)?),
        "TEXT[]" | "VARCHAR[]" => JsValue::Array(row.try_get::<Vec<String>, _>(i)?.into_iter().map(JsValue::String).collect()),
        "UUID[]" => JsValue::Array(row.try_get::<Vec<uuid::Uuid>, _>(i)?.into_iter().map(|u| JsValue::String(u.to_string())).collect()),
        "INT4[]" => JsValue::Array(row.try_get::<Vec<i32>, _>(i)?.into_iter().map(|n| JsValue::Number(f64::from(n))).collect()),
        other => {
            return Err(sqlx::Error::Decode(format!("pgjson: unsupported column type {other}").into()));
        }
    })
}

/// A row as a JS object (keys in SELECT order).
pub fn row(row: &PgRow) -> Result<JsValue, sqlx::Error> {
    let mut out = Vec::with_capacity(row.len());
    for (i, c) in row.columns().iter().enumerate() {
        out.push((c.name().to_string(), column(row, i)?));
    }
    Ok(JsValue::Object(out))
}

/// `query(sql, params)` → array of row objects.
pub fn rows(rows: &[PgRow]) -> Result<JsValue, sqlx::Error> {
    Ok(JsValue::Array(rows.iter().map(row).collect::<Result<_, _>>()?))
}

/// `JSON.stringify(value, null, 2)`.
pub fn stringify_pretty(v: &JsValue) -> String {
    let mut out = String::new();
    write(&mut out, v, 0);
    out
}

fn indent(out: &mut String, level: usize) {
    out.push('\n');
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn write(out: &mut String, v: &JsValue, level: usize) {
    match v {
        JsValue::Array(a) if !a.is_empty() => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                indent(out, level + 1);
                write(out, x, level + 1);
            }
            indent(out, level);
            out.push(']');
        }
        JsValue::Object(m) if !m.is_empty() => {
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                indent(out, level + 1);
                out.push_str(&oc_js::stringify_str(k));
                out.push_str(": ");
                write(out, x, level + 1);
            }
            indent(out, level);
            out.push('}');
        }
        other => out.push_str(&other.stringify()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_like_json_stringify() {
        let v = JsValue::parse(r#"{"a":1,"b":[],"c":{},"d":[1,{"e":null}],"f":"x\ny"}"#).unwrap();
        assert_eq!(
            stringify_pretty(&v),
            "{\n  \"a\": 1,\n  \"b\": [],\n  \"c\": {},\n  \"d\": [\n    1,\n    {\n      \"e\": null\n    }\n  ],\n  \"f\": \"x\\ny\"\n}"
        );
        assert_eq!(stringify_pretty(&JsValue::Null), "null");
        assert_eq!(stringify_pretty(&JsValue::Array(vec![])), "[]");
    }
}
