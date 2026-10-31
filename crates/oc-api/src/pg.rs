//! `query(sql, params)` / `one(sql, params)` of `@oc/db`, with rows shaped like node-postgres hands them to
//! the TS routes (many routes return rows as they come: `return { designs: rows }`).
//!
//! Row values: `int2`/`int4`/`oid`/`float4`/`float8` → number, `int8`/`numeric` → string, `bool` → boolean,
//! text-like/`uuid`/`inet` → string, `json`/`jsonb` → parsed, `timestamptz`/`timestamp`/`date` → `Date` (its
//! ISO string, millisecond precision), `bytea` → `Buffer` (`{"type":"Buffer","data":[…]}`), arrays of those →
//! arrays, NULL → null. Keys keep the SELECT order.
//!
//! Parameters: node-postgres sends text and lets Postgres infer the type; sqlx binds typed values. Ports
//! bind [`Arg::Text`] for a string the TS passed through and cast in SQL (`$1::uuid`) where the column is a
//! uuid, which keeps the TS behaviour for malformed input (Postgres rejects it → 500).

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde_json::{Map, Value, json};
use sqlx::postgres::{PgArguments, PgRow};
use sqlx::query::Query;
use sqlx::{Column, Executor, Postgres, Row, TypeInfo, ValueRef};
use uuid::Uuid;

use crate::http::jsjson;

/// A bind parameter.
#[derive(Debug, Clone)]
pub enum Arg {
    Text(Option<String>),
    Uuid(Option<Uuid>),
    Int(Option<i64>),
    Int4(Option<i32>),
    Float(Option<f64>),
    Bool(Option<bool>),
    /// A JS value passed for a json/jsonb column (bound as jsonb).
    Json(Option<Value>),
    TextArray(Option<Vec<String>>),
    Bytes(Option<Vec<u8>>),
    Time(Option<DateTime<Utc>>),
}

pub fn text(s: impl Into<String>) -> Arg {
    Arg::Text(Some(s.into()))
}
pub fn opt_text(s: Option<impl Into<String>>) -> Arg {
    Arg::Text(s.map(Into::into))
}
pub fn uuid(u: Uuid) -> Arg {
    Arg::Uuid(Some(u))
}

pub fn bind<'q>(mut q: Query<'q, Postgres, PgArguments>, args: &'q [Arg]) -> Query<'q, Postgres, PgArguments> {
    for a in args {
        q = match a {
            Arg::Text(v) => q.bind(v.as_deref()),
            Arg::Uuid(v) => q.bind(*v),
            Arg::Int(v) => q.bind(*v),
            Arg::Int4(v) => q.bind(*v),
            Arg::Float(v) => q.bind(*v),
            Arg::Bool(v) => q.bind(*v),
            Arg::Json(v) => q.bind(v.as_ref().map(sqlx::types::Json)),
            Arg::TextArray(v) => q.bind(v.as_deref()),
            Arg::Bytes(v) => q.bind(v.as_deref()),
            Arg::Time(v) => q.bind(*v),
        };
    }
    q
}

/// `query(sql, params)`: every row.
pub async fn rows<'e, E: Executor<'e, Database = Postgres>>(ex: E, sql: &str, args: &[Arg]) -> Result<Vec<PgRow>, sqlx::Error> {
    bind(sqlx::query(sql), args).fetch_all(ex).await
}

/// `one(sql, params)`: the first row or `None`.
pub async fn one<'e, E: Executor<'e, Database = Postgres>>(ex: E, sql: &str, args: &[Arg]) -> Result<Option<PgRow>, sqlx::Error> {
    bind(sqlx::query(sql), args).fetch_optional(ex).await
}

/// `query(sql, params)` for its side effect; returns the affected row count.
pub async fn exec<'e, E: Executor<'e, Database = Postgres>>(ex: E, sql: &str, args: &[Arg]) -> Result<u64, sqlx::Error> {
    Ok(bind(sqlx::query(sql), args).execute(ex).await?.rows_affected())
}

/// `query(sql, params)` as node-postgres rows.
pub async fn json_rows<'e, E: Executor<'e, Database = Postgres>>(ex: E, sql: &str, args: &[Arg]) -> Result<Vec<Value>, sqlx::Error> {
    rows(ex, sql, args).await?.iter().map(row).collect()
}

/// `one(sql, params)` as a node-postgres row.
pub async fn json_one<'e, E: Executor<'e, Database = Postgres>>(ex: E, sql: &str, args: &[Arg]) -> Result<Option<Value>, sqlx::Error> {
    one(ex, sql, args).await?.as_ref().map(row).transpose()
}

fn date(t: DateTime<Utc>) -> Value {
    Value::String(oc_js::iso_string(&t))
}

fn buffer(b: &[u8]) -> Value {
    json!({ "type": "Buffer", "data": b })
}

fn float(f: f64) -> Value {
    jsjson::number_value(f).unwrap_or(Value::Null)
}

/// One column as node-postgres returns it.
pub fn column(r: &PgRow, i: usize) -> Result<Value, sqlx::Error> {
    if r.try_get_raw(i)?.is_null() {
        return Ok(Value::Null);
    }
    let ty = r.column(i).type_info().name().to_ascii_uppercase();
    Ok(match ty.as_str() {
        "UUID" => Value::String(r.try_get::<Uuid, _>(i)?.to_string()),
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CHAR" | "CITEXT" => Value::String(r.try_get::<String, _>(i)?),
        "INET" | "CIDR" => Value::String(inet_text(r.try_get_raw(i)?.as_bytes().map_err(sqlx::Error::Decode)?)),
        "INT2" => json!(r.try_get::<i16, _>(i)?),
        "INT4" => json!(r.try_get::<i32, _>(i)?),
        "OID" => json!(r.try_get::<sqlx::postgres::types::Oid, _>(i)?.0),
        "INT8" => Value::String(r.try_get::<i64, _>(i)?.to_string()),
        "NUMERIC" => Value::String(numeric_text(r.try_get_raw(i)?.as_bytes().map_err(sqlx::Error::Decode)?)),
        // Postgres prints a real with the shortest round-trip digits; node-postgres parses that text.
        "FLOAT4" => float(r.try_get::<f32, _>(i)?.to_string().parse().unwrap_or(f64::NAN)),
        "FLOAT8" => float(r.try_get::<f64, _>(i)?),
        "BOOL" => Value::Bool(r.try_get::<bool, _>(i)?),
        "TIMESTAMPTZ" => date(r.try_get::<DateTime<Utc>, _>(i)?),
        // node-postgres reads a timestamp without time zone (and a date) as local time; the servers run in UTC.
        "TIMESTAMP" => date(r.try_get::<NaiveDateTime, _>(i)?.and_utc()),
        "DATE" => date(r.try_get::<NaiveDate, _>(i)?.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc()),
        "JSON" | "JSONB" => r.try_get::<Value, _>(i)?,
        "BYTEA" => buffer(&r.try_get::<Vec<u8>, _>(i)?),
        "TEXT[]" | "VARCHAR[]" | "NAME[]" => Value::Array(r.try_get::<Vec<String>, _>(i)?.into_iter().map(Value::String).collect()),
        "UUID[]" => Value::Array(r.try_get::<Vec<Uuid>, _>(i)?.into_iter().map(|u| Value::String(u.to_string())).collect()),
        "INT4[]" => Value::Array(r.try_get::<Vec<i32>, _>(i)?.into_iter().map(|n| json!(n)).collect()),
        "INT2[]" => Value::Array(r.try_get::<Vec<i16>, _>(i)?.into_iter().map(|n| json!(n)).collect()),
        "INT8[]" => Value::Array(r.try_get::<Vec<i64>, _>(i)?.into_iter().map(|n| Value::String(n.to_string())).collect()),
        "FLOAT8[]" => Value::Array(r.try_get::<Vec<f64>, _>(i)?.into_iter().map(float).collect()),
        "BOOL[]" => Value::Array(r.try_get::<Vec<bool>, _>(i)?.into_iter().map(Value::Bool).collect()),
        "JSONB[]" | "JSON[]" => Value::Array(r.try_get::<Vec<Value>, _>(i)?),
        "TIMESTAMPTZ[]" => Value::Array(r.try_get::<Vec<DateTime<Utc>>, _>(i)?.into_iter().map(date).collect()),
        other => return Err(sqlx::Error::Decode(format!("pg: unsupported column type {other}").into())),
    })
}

/// Postgres' text output of a binary `inet`/`cidr` (a host address prints without its `/32` or `/128`).
fn inet_text(b: &[u8]) -> String {
    if b.len() < 4 {
        return String::new();
    }
    let (family, bits, is_cidr, addr) = (b[0], b[1], b[2], &b[4..]);
    let (ip, max) = if family == 2 && addr.len() >= 4 {
        (std::net::IpAddr::from([addr[0], addr[1], addr[2], addr[3]]), 32)
    } else if addr.len() >= 16 {
        let mut a = [0u8; 16];
        a.copy_from_slice(&addr[..16]);
        (std::net::IpAddr::from(a), 128)
    } else {
        return String::new();
    };
    if bits == max && is_cidr == 0 { ip.to_string() } else { format!("{ip}/{bits}") }
}

/// Postgres' text output of a binary `numeric`.
fn numeric_text(b: &[u8]) -> String {
    let u16_at = |i: usize| u16::from_be_bytes([b.get(i).copied().unwrap_or(0), b.get(i + 1).copied().unwrap_or(0)]);
    if b.len() < 8 {
        return "0".into();
    }
    let ndigits = u16_at(0) as usize;
    let weight = u16_at(2) as i16 as i32;
    let sign = u16_at(4);
    let dscale = u16_at(6) as usize;
    if sign == 0xC000 {
        return "NaN".into();
    }
    let digits: Vec<u16> = (0..ndigits).map(|k| u16_at(8 + 2 * k)).collect();
    let mut int_part = String::new();
    for w in 0..=weight.max(-1) {
        let d = digits.get(w as usize).copied().unwrap_or(0);
        if int_part.is_empty() {
            int_part = d.to_string();
        } else {
            int_part.push_str(&format!("{d:04}"));
        }
    }
    if int_part.is_empty() {
        int_part = "0".into();
    }
    let mut frac = String::new();
    let mut k = weight + 1;
    while frac.len() < dscale {
        let d = if k >= 0 { digits.get(k as usize).copied().unwrap_or(0) } else { 0 };
        frac.push_str(&format!("{d:04}"));
        k += 1;
    }
    frac.truncate(dscale);
    let neg = if sign == 0x4000 { "-" } else { "" };
    if dscale > 0 { format!("{neg}{int_part}.{frac}") } else { format!("{neg}{int_part}") }
}

/// A row as a JS object.
pub fn row(r: &PgRow) -> Result<Value, sqlx::Error> {
    let mut out = Map::new();
    for (i, c) in r.columns().iter().enumerate() {
        out.insert(c.name().to_owned(), column(r, i)?);
    }
    Ok(Value::Object(out))
}

/// `Number(x)` of a count/sum (int8/numeric arrive as strings).
pub fn number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse().unwrap_or(f64::NAN),
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => f64::NAN,
    }
}
