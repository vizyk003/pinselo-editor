//! JSON as JavaScript sees it.
//!
//! * [`parse`]: `JSON.parse(text)` including V8's error messages (Fastify answers a malformed JSON body with
//!   `400 {"error": <the SyntaxError message>}`, so the message is part of the HTTP contract).
//! * [`stringify`]: `JSON.stringify(value)` of a `serde_json::Value` (numbers printed the JS way: `1`, not `1.0`).
//!
//! The error messages follow V8's `JsonParser` (`src/json/json-parser.cc`, Node 22/24): the same token
//! classes, the same "expected …" messages at the same positions, the same 10-character context window for
//! "Unexpected token" errors. Positions, lines and columns count UTF-16 code units like V8.

use serde_json::{Map, Number, Value};

/// `JSON.parse(text)`; `Err` carries the `SyntaxError` message.
pub fn parse(text: &str) -> Result<Value, String> {
    let src: Vec<u16> = text.encode_utf16().collect();
    let mut p = Parser { src: &src, pos: 0 };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    if p.pos < src.len() {
        let (line, col) = p.location();
        return Err(format!("Unexpected non-whitespace character after JSON at position {} (line {line} column {col})", p.pos));
    }
    Ok(v)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tok {
    Str,
    Num,
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    Colon,
    Comma,
    True,
    False,
    Null,
    Ws,
    Illegal,
    Eos,
}

fn tok_of(c: Option<u16>) -> Tok {
    let Some(c) = c else { return Tok::Eos };
    match c {
        0x22 => Tok::Str,
        0x2d | 0x30..=0x39 => Tok::Num,
        0x7b => Tok::LBrace,
        0x7d => Tok::RBrace,
        0x5b => Tok::LBrack,
        0x5d => Tok::RBrack,
        0x3a => Tok::Colon,
        0x2c => Tok::Comma,
        0x74 => Tok::True,
        0x66 => Tok::False,
        0x6e => Tok::Null,
        0x20 | 0x09 | 0x0a | 0x0d => Tok::Ws,
        _ => Tok::Illegal,
    }
}

struct Parser<'a> {
    src: &'a [u16],
    pos: usize,
}

impl Parser<'_> {
    fn peek_char(&self) -> Option<u16> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek_char(), Some(0x20 | 0x09 | 0x0a | 0x0d)) {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Tok {
        self.skip_ws();
        tok_of(self.peek_char())
    }

    /// `<message> in JSON at position P (line L column C)`.
    fn at(&self, message: &str) -> String {
        let (line, col) = self.location();
        format!("{message} in JSON at position {} (line {line} column {col})", self.pos)
    }

    fn location(&self) -> (usize, usize) {
        let mut line = 1;
        let mut last_break = 0usize;
        let mut i = 0usize;
        let end = self.pos.min(self.src.len());
        while i < end {
            if self.src[i] == 0x0d && i + 1 < end && self.src[i + 1] == 0x0a {
                i += 1;
            }
            if self.src[i] == 0x0d || self.src[i] == 0x0a {
                line += 1;
                last_break = i + 1;
            }
            i += 1;
        }
        (line, 1 + i - last_break)
    }

    /// V8 `ReportUnexpectedToken(token)` without an explicit message.
    fn unexpected(&self, tok: Tok) -> String {
        match tok {
            Tok::Eos => "Unexpected end of JSON input".to_owned(),
            Tok::Num => self.at("Unexpected number"),
            Tok::Str => self.at("Unexpected string"),
            _ => {
                let whole = String::from_utf16_lossy(self.src);
                if matches!(whole.as_str(), "[object Object]" | "undefined" | "Infinity" | "NaN" | "-Infinity") {
                    return format!("\"{whole}\" is not valid JSON");
                }
                let ch = String::from_utf16_lossy(&self.src[self.pos..self.pos + 1]);
                let len = self.src.len();
                const CTX: usize = 10;
                if len <= CTX * 2 + 1 {
                    format!("Unexpected token '{ch}', \"{whole}\" is not valid JSON")
                } else if self.pos < CTX {
                    let s = String::from_utf16_lossy(&self.src[..(self.pos + CTX).min(len)]);
                    format!("Unexpected token '{ch}', \"{s}\"... is not valid JSON")
                } else if self.pos < len - CTX {
                    let s = String::from_utf16_lossy(&self.src[self.pos - CTX..self.pos + CTX]);
                    format!("Unexpected token '{ch}', ...\"{s}\"... is not valid JSON")
                } else {
                    let s = String::from_utf16_lossy(&self.src[self.pos - CTX..]);
                    format!("Unexpected token '{ch}', ...\"{s}\" is not valid JSON")
                }
            }
        }
    }

    /// V8 `ExpectNext(token, message)`: whitespace skipped; a mismatch reports `message` at the cursor.
    fn expect(&mut self, want: Tok, message: &str) -> Result<(), String> {
        if self.peek() == want {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.at(message))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.peek() {
            Tok::Str => Ok(Value::String(self.string()?)),
            Tok::Num => self.number(),
            Tok::LBrace => self.object(),
            Tok::LBrack => self.array(),
            Tok::True => self.literal("true").map(|_| Value::Bool(true)),
            Tok::False => self.literal("false").map(|_| Value::Bool(false)),
            Tok::Null => self.literal("null").map(|_| Value::Null),
            t => Err(self.unexpected(t)),
        }
    }

    fn literal(&mut self, word: &str) -> Result<(), String> {
        let w: Vec<u16> = word.encode_utf16().collect();
        // The first character was matched by the token class.
        self.pos += 1;
        for &c in &w[1..] {
            match self.peek_char() {
                None => return Err(self.unexpected(Tok::Eos)),
                Some(x) if x != c => return Err(self.unexpected(tok_of(Some(x)))),
                Some(_) => self.pos += 1,
            }
        }
        Ok(())
    }

    fn object(&mut self) -> Result<Value, String> {
        self.pos += 1; // '{'
        let mut map = Map::new();
        if self.peek() == Tok::RBrace {
            self.pos += 1;
            return Ok(Value::Object(map));
        }
        if self.peek() != Tok::Str {
            return Err(self.at("Expected property name or '}'"));
        }
        loop {
            let key = self.string()?;
            self.expect(Tok::Colon, "Expected ':' after property name")?;
            let v = self.value()?;
            // JSON.parse: a repeated key keeps its first position and takes the last value.
            map.insert(key, v);
            match self.peek() {
                Tok::Comma => {
                    self.pos += 1;
                    if self.peek() != Tok::Str {
                        return Err(self.at("Expected double-quoted property name"));
                    }
                }
                Tok::RBrace => {
                    self.pos += 1;
                    return Ok(Value::Object(map));
                }
                _ => return Err(self.at("Expected ',' or '}' after property value")),
            }
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.pos += 1; // '['
        let mut out = Vec::new();
        if self.peek() == Tok::RBrack {
            self.pos += 1;
            return Ok(Value::Array(out));
        }
        loop {
            out.push(self.value()?);
            match self.peek() {
                Tok::Comma => self.pos += 1,
                Tok::RBrack => {
                    self.pos += 1;
                    return Ok(Value::Array(out));
                }
                _ => return Err(self.at("Expected ',' or ']' after array element")),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1; // '"'
        let mut out: Vec<u16> = Vec::new();
        loop {
            let Some(c) = self.peek_char() else {
                return Err(self.at("Unterminated string"));
            };
            match c {
                0x22 => {
                    self.pos += 1;
                    return Ok(String::from_utf16_lossy(&out));
                }
                0x5c => {
                    self.pos += 1;
                    let Some(e) = self.peek_char() else {
                        return Err(self.unexpected(Tok::Eos));
                    };
                    let decoded = match e {
                        0x22 => 0x22,
                        0x5c => 0x5c,
                        0x2f => 0x2f,
                        0x62 => 0x08,
                        0x66 => 0x0c,
                        0x6e => 0x0a,
                        0x72 => 0x0d,
                        0x74 => 0x09,
                        0x75 => {
                            let mut v: u16 = 0;
                            for _ in 0..4 {
                                self.pos += 1;
                                let d = self.peek_char().and_then(|h| char::from_u32(u32::from(h))).and_then(|h| h.to_digit(16));
                                match d {
                                    Some(d) => v = v * 16 + d as u16,
                                    None => return Err(self.at("Bad Unicode escape")),
                                }
                            }
                            v
                        }
                        _ => return Err(self.at("Bad escaped character")),
                    };
                    out.push(decoded);
                    self.pos += 1;
                }
                c if c < 0x20 => return Err(self.at("Bad control character in string literal")),
                c => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn is_digit(&self) -> bool {
        matches!(self.peek_char(), Some(0x30..=0x39))
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek_char() == Some(0x2d) {
            self.pos += 1;
            if !self.is_digit() {
                return Err(self.at("No number after minus sign"));
            }
        }
        if self.peek_char() == Some(0x30) {
            self.pos += 1;
            if self.is_digit() {
                return Err(self.unexpected(Tok::Num));
            }
        } else {
            while self.is_digit() {
                self.pos += 1;
            }
        }
        let mut float = false;
        if self.peek_char() == Some(0x2e) {
            float = true;
            self.pos += 1;
            if !self.is_digit() {
                return Err(self.at("Unterminated fractional number"));
            }
            while self.is_digit() {
                self.pos += 1;
            }
        }
        if matches!(self.peek_char(), Some(0x65 | 0x45)) {
            float = true;
            self.pos += 1;
            if matches!(self.peek_char(), Some(0x2b | 0x2d)) {
                self.pos += 1;
            }
            if !self.is_digit() {
                return Err(self.at("Exponent part is missing a number"));
            }
            while self.is_digit() {
                self.pos += 1;
            }
        }
        let text = String::from_utf16_lossy(&self.src[start..self.pos]);
        Ok(js_number(&text, float))
    }
}

/// A JSON number literal as a JS double, kept as an integer `Number` when it is an integer within the safe
/// range (so it prints without a fraction and binds as an integer).
fn js_number(text: &str, float: bool) -> Value {
    let f: f64 = text.parse().unwrap_or(f64::NAN);
    let _ = float;
    number_value(f).unwrap_or(Value::Null)
}

/// A JS number as a JSON value (`None` for NaN/±Infinity, which JSON cannot carry).
pub fn number_value(f: f64) -> Option<Value> {
    if !f.is_finite() {
        return None;
    }
    if f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0 {
        if f == 0.0 {
            return Some(Value::Number(0.into()));
        }
        return Some(Value::Number((f as i64).into()));
    }
    Number::from_f64(f).map(Value::Number)
}

/// `JSON.stringify(value)`.
pub fn stringify(v: &Value) -> String {
    let mut out = String::new();
    write(&mut out, v);
    out
}

fn write(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number_text(n)),
        Value::String(s) => out.push_str(&oc_js::stringify_str(s)),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(out, x);
            }
            out.push(']');
        }
        Value::Object(m) => {
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&oc_js::stringify_str(k));
                out.push(':');
                write(out, x);
            }
            out.push('}');
        }
    }
}

/// `String(n)` for a JSON number.
pub fn number_text(n: &Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return oc_js::number_to_string(u as f64);
    }
    oc_js::number_to_string(n.as_f64().unwrap_or(f64::NAN))
}

/// The JS number of a JSON value (`Number(x)` for numbers only).
pub fn as_f64(v: &Value) -> Option<f64> {
    v.as_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stringify_numbers_like_js() {
        let v: Value = serde_json::from_str(r#"{"a":1.0,"b":1.5,"c":-0.0,"d":1e21,"e":[1,"x"]}"#).unwrap();
        assert_eq!(stringify(&v), r#"{"a":1,"b":1.5,"c":0,"d":1e+21,"e":[1,"x"]}"#);
    }

    /// Expected messages printed by Node 22 (`JSON.parse`).
    #[test]
    fn v8_messages() {
        let cases: &[(&str, &str)] = &[
            ("{\"email\": x}", "Unexpected token 'x', \"{\"email\": x}\" is not valid JSON"),
            ("{\"email\":", "Unexpected end of JSON input"),
            ("", "Unexpected end of JSON input"),
            ("{", "Expected property name or '}' in JSON at position 1 (line 1 column 2)"),
            ("{a:1}", "Expected property name or '}' in JSON at position 1 (line 1 column 2)"),
            ("{\"a\" 1}", "Expected ':' after property name in JSON at position 5 (line 1 column 6)"),
            ("{\"a\":1 \"b\":2}", "Expected ',' or '}' after property value in JSON at position 7 (line 1 column 8)"),
            ("{\"a\":1,}", "Expected double-quoted property name in JSON at position 7 (line 1 column 8)"),
            ("[1 2]", "Expected ',' or ']' after array element in JSON at position 3 (line 1 column 4)"),
            ("[1,]", "Unexpected token ']', \"[1,]\" is not valid JSON"),
            ("{} x", "Unexpected non-whitespace character after JSON at position 3 (line 1 column 4)"),
            ("\"abc", "Unterminated string in JSON at position 4 (line 1 column 5)"),
            ("\"a\\x\"", "Bad escaped character in JSON at position 3 (line 1 column 4)"),
            ("\"a\nb\"", "Bad control character in string literal in JSON at position 2 (line 1 column 3)"),
            ("-", "No number after minus sign in JSON at position 1 (line 1 column 2)"),
            ("01", "Unexpected number in JSON at position 1 (line 1 column 2)"),
            ("1.", "Unterminated fractional number in JSON at position 2 (line 1 column 3)"),
            ("1e", "Exponent part is missing a number in JSON at position 2 (line 1 column 3)"),
            ("tru", "Unexpected end of JSON input"),
            ("trux", "Unexpected token 'x', \"trux\" is not valid JSON"),
            ("undefined", "\"undefined\" is not valid JSON"),
            ("{\"email\": \"a@b.c\", \"password\": nope}", "Unexpected token 'o', ...\"ssword\": nope}\" is not valid JSON"),
            ("{\n  \"a\": 1,\n  \"b\": x\n}", "Unexpected token 'x', ...\"1,\n  \"b\": x\n}\" is not valid JSON"),
            ("{\"a\":1}\n\n}", "Unexpected non-whitespace character after JSON at position 9 (line 3 column 1)"),
            ("\"\\u12G4\"", "Bad Unicode escape in JSON at position 5 (line 1 column 6)"),
            ("\"\\u12", "Bad Unicode escape in JSON at position 5 (line 1 column 6)"),
            ("[", "Unexpected end of JSON input"),
            ("[1", "Expected ',' or ']' after array element in JSON at position 2 (line 1 column 3)"),
            ("{\"a\"", "Expected ':' after property name in JSON at position 4 (line 1 column 5)"),
            ("1.e5", "Unterminated fractional number in JSON at position 2 (line 1 column 3)"),
            ("{\"a\":01}", "Unexpected number in JSON at position 6 (line 1 column 7)"),
            ("{\"a\":1,\"b\"}", "Expected ':' after property name in JSON at position 10 (line 1 column 11)"),
            ("\"\\", "Unexpected end of JSON input"),
            ("NaN", "\"NaN\" is not valid JSON"),
            ("t1", "Unexpected number in JSON at position 1 (line 1 column 2)"),
            ("f\"", "Unexpected string in JSON at position 1 (line 1 column 2)"),
            ("{\"long_key_name_here\": [1, 2, 3, }", "Unexpected token '}', ...\"[1, 2, 3, }\" is not valid JSON"),
        ];
        for (input, want) in cases {
            assert_eq!(parse(input).unwrap_err(), *want, "input {input:?}");
        }
    }
}
