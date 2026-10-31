// SPDX-License-Identifier: AGPL-3.0-only
//! Formula lexer — HyperFormula's chevrotain token set, matched in the same order (first pattern that
//! matches wins, not the longest), with its whitespace post-processing.

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ws,
    Plus,
    Minus,
    Times,
    Div,
    Pow,
    Eq,
    Ne,
    Percent,
    Ge,
    Le,
    Gt,
    Lt,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Offset,
    /// Function name including the trailing `(` in the image.
    Proc,
    RangeSep,
    ColRange,
    RowRange,
    Number,
    Str,
    ErrorLit,
    Amp,
    CellRef,
    Named,
    Semi,
    Comma,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: Tok,
    pub image: String,
    /// Leading whitespace (bound after lexing).
    pub ws: Option<String>,
}

pub fn is_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{02AF}').contains(&c)
}

pub fn is_name_char(c: char) -> bool {
    is_letter(c) || c.is_ascii_digit() || c == '_' || c == '.'
}

fn is_sheet_char(c: char) -> bool {
    is_letter(c) || c.is_ascii_digit() || c == '_'
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `(UNQUOTED|QUOTED)!` at `i` → end index (after `!`).
fn match_sheet_prefix(s: &[char], i: usize) -> Option<usize> {
    if i >= s.len() {
        return None;
    }
    if s[i] == '\'' {
        let mut j = i + 1;
        loop {
            if j >= s.len() {
                return None;
            }
            let c = s[j];
            if c != '\'' && !is_line_terminator(c) {
                j += 1;
            } else if c == '\'' && s.get(j + 1) == Some(&'\'') {
                j += 2;
            } else {
                break;
            }
        }
        if s.get(j) != Some(&'\'') {
            return None;
        }
        j += 1;
        return (s.get(j) == Some(&'!')).then_some(j + 1);
    }
    let mut j = i;
    while j < s.len() && is_sheet_char(s[j]) {
        j += 1;
    }
    (j > i && s.get(j) == Some(&'!')).then_some(j + 1)
}

/// `\$?[A-Za-z]+` → end.
fn match_col(s: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    if s.get(j) == Some(&'$') {
        j += 1;
    }
    let st = j;
    while j < s.len() && s[j].is_ascii_alphabetic() {
        j += 1;
    }
    (j > st).then_some(j)
}

/// `\$?[0-9]+` → end.
fn match_row(s: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    if s.get(j) == Some(&'$') {
        j += 1;
    }
    let st = j;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
    }
    (j > st).then_some(j)
}

/// `(SHEET!)?<part>` trying the sheet prefix first (regex backtracking order).
fn with_sheet(s: &[char], i: usize, part: impl Fn(&[char], usize) -> Option<usize>) -> Option<usize> {
    if let Some(j) = match_sheet_prefix(s, i)
        && let Some(e) = part(s, j)
    {
        return Some(e);
    }
    part(s, i)
}

fn match_range(s: &[char], i: usize, part: &dyn Fn(&[char], usize) -> Option<usize>) -> Option<usize> {
    let attempt = |start: usize| -> Option<usize> {
        let a = part(s, start)?;
        if s.get(a) != Some(&':') {
            return None;
        }
        with_sheet(s, a + 1, part)
    };
    if let Some(j) = match_sheet_prefix(s, i)
        && let Some(e) = attempt(j)
    {
        return Some(e);
    }
    attempt(i)
}

fn match_cell_ref(s: &[char], i: usize) -> Option<usize> {
    let body = |s: &[char], j: usize| -> Option<usize> {
        let a = match_col(s, j)?;
        let b = match_row(s, a)?;
        // Next character must not be a name character (end of input counts as '@').
        match s.get(b) {
            Some(c) if is_name_char(*c) => None,
            _ => Some(b),
        }
    };
    with_sheet(s, i, body)
}

fn match_number(s: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    if s.get(j) == Some(&'.') {
        j += 1;
        let st = j;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
        }
        if j == st {
            return None;
        }
    } else {
        let st = j;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
        }
        if j == st {
            return None;
        }
        if s.get(j) == Some(&'.') {
            j += 1;
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
            }
        }
    }
    if s.get(j) == Some(&'e') {
        let mut k = j + 1;
        if matches!(s.get(k), Some('+') | Some('-')) {
            k += 1;
        }
        let st = k;
        while k < s.len() && s[k].is_ascii_digit() {
            k += 1;
        }
        if k > st {
            j = k;
        }
    }
    Some(j)
}

fn match_string(s: &[char], i: usize) -> Option<usize> {
    if s.get(i) != Some(&'"') {
        return None;
    }
    let mut j = i + 1;
    loop {
        match s.get(j) {
            None => return None,
            Some('"') => return Some(j + 1),
            Some('\\') => match s.get(j + 1) {
                Some(c) if !is_line_terminator(*c) => j += 2,
                _ => return None,
            },
            Some(_) => j += 1,
        }
    }
}

fn match_error(s: &[char], i: usize) -> Option<usize> {
    if s.get(i) != Some(&'#') {
        return None;
    }
    let mut j = i + 1;
    while j < s.len() && (s[j].is_ascii_alphanumeric() || s[j] == '/') {
        j += 1;
    }
    if j == i + 1 {
        return None;
    }
    if matches!(s.get(j), Some('?') | Some('!')) {
        j += 1;
    }
    Some(j)
}

fn match_named(s: &[char], i: usize) -> Option<usize> {
    let c = *s.get(i)?;
    if !(is_letter(c) || c == '_') {
        return None;
    }
    let mut j = i + 1;
    while j < s.len() && is_name_char(s[j]) {
        j += 1;
    }
    // Reject R1C1-looking names (`^[rR][0-9]*[cC][0-9]*$`).
    let w = &s[i..j];
    if matches!(w.first(), Some('r') | Some('R')) {
        let mut k = 1;
        while k < w.len() && w[k].is_ascii_digit() {
            k += 1;
        }
        if matches!(w.get(k), Some('c') | Some('C')) {
            k += 1;
            while k < w.len() && w[k].is_ascii_digit() {
                k += 1;
            }
            if k == w.len() {
                return None;
            }
        }
    }
    Some(j)
}

fn match_proc(s: &[char], i: usize) -> Option<usize> {
    let c = *s.get(i)?;
    if !is_letter(c) {
        return None;
    }
    let mut j = i + 1;
    while j < s.len() && is_name_char(s[j]) {
        j += 1;
    }
    (s.get(j) == Some(&'(')).then_some(j + 1)
}

fn match_offset(s: &[char], i: usize) -> Option<usize> {
    let w: String = s.get(i..i + 6)?.iter().collect();
    w.eq_ignore_ascii_case("offset").then_some(i + 6)
}

/// A formula HyperFormula's lexer or parser rejects (the cell evaluates to `#ERROR!`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxError;

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("formula syntax error")
    }
}

impl std::error::Error for SyntaxError {}

/// Tokenize; [`SyntaxError`] on a lexing error (any character no token matches).
pub fn tokenize(text: &str) -> Result<Vec<Token>, SyntaxError> {
    let s: Vec<char> = text.chars().collect();
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0;
    let mut failed = false;
    while i < s.len() {
        let c = s[i];
        let single = |k: Tok| Some((k, i + 1));
        let m: Option<(Tok, usize)> = if matches!(c, ' ' | '\t' | '\n' | '\r') {
            let mut j = i;
            while j < s.len() && matches!(s[j], ' ' | '\t' | '\n' | '\r') {
                j += 1;
            }
            Some((Tok::Ws, j))
        } else if c == '+' {
            single(Tok::Plus)
        } else if c == '-' {
            single(Tok::Minus)
        } else if c == '*' {
            single(Tok::Times)
        } else if c == '/' {
            single(Tok::Div)
        } else if c == '^' {
            single(Tok::Pow)
        } else if c == '=' {
            single(Tok::Eq)
        } else if c == '<' && s.get(i + 1) == Some(&'>') {
            Some((Tok::Ne, i + 2))
        } else if c == '%' {
            single(Tok::Percent)
        } else if c == '>' && s.get(i + 1) == Some(&'=') {
            Some((Tok::Ge, i + 2))
        } else if c == '<' && s.get(i + 1) == Some(&'=') {
            Some((Tok::Le, i + 2))
        } else if c == '>' {
            single(Tok::Gt)
        } else if c == '<' {
            single(Tok::Lt)
        } else if c == '(' {
            single(Tok::LParen)
        } else if c == ')' {
            single(Tok::RParen)
        } else if c == '{' {
            single(Tok::LBrace)
        } else if c == '}' {
            single(Tok::RBrace)
        } else if let Some(j) = match_offset(&s, i) {
            Some((Tok::Offset, j))
        } else if let Some(j) = match_proc(&s, i) {
            Some((Tok::Proc, j))
        } else if c == ':' {
            single(Tok::RangeSep)
        } else if let Some(j) = match_range(&s, i, &match_col) {
            Some((Tok::ColRange, j))
        } else if let Some(j) = match_range(&s, i, &match_row) {
            Some((Tok::RowRange, j))
        } else if let Some(j) = match_number(&s, i) {
            Some((Tok::Number, j))
        } else if let Some(j) = match_string(&s, i) {
            Some((Tok::Str, j))
        } else if let Some(j) = match_error(&s, i) {
            Some((Tok::ErrorLit, j))
        } else if c == '&' {
            single(Tok::Amp)
        } else if let Some(j) = match_cell_ref(&s, i) {
            Some((Tok::CellRef, j))
        } else if let Some(j) = match_named(&s, i) {
            Some((Tok::Named, j))
        } else if c == ';' {
            single(Tok::Semi)
        } else if c == ',' {
            single(Tok::Comma)
        } else {
            None
        };
        match m {
            Some((kind, j)) => {
                out.push(Token { kind, image: s[i..j].iter().collect(), ws: None });
                i = j;
            }
            None => {
                failed = true;
                i += 1;
            }
        }
    }
    if failed {
        return Err(SyntaxError);
    }
    // trimTrailingWhitespaces
    if out.last().is_some_and(|t| t.kind == Tok::Ws) {
        out.pop();
    }
    let out = filter_neighbors(out, |p, c, n| {
        matches!(p.kind, Tok::CellRef | Tok::RangeSep) && c.kind == Tok::Ws && matches!(n.kind, Tok::CellRef | Tok::RangeSep)
    });
    let out = filter_neighbors(out, |p, c, n| p.kind != Tok::Comma && c.kind == Tok::Ws && n.kind == Tok::Comma);
    // bindWhitespacesToTokens
    let mut bound: Vec<Token> = Vec::with_capacity(out.len());
    for (i, t) in out.iter().enumerate() {
        if t.kind == Tok::Ws {
            continue;
        }
        let mut t = t.clone();
        if i > 0 && out[i - 1].kind == Tok::Ws {
            t.ws = Some(out[i - 1].image.clone());
        }
        bound.push(t);
    }
    Ok(bound)
}

fn filter_neighbors(tokens: Vec<Token>, skip: impl Fn(&Token, &Token, &Token) -> bool) -> Vec<Token> {
    if tokens.len() < 3 {
        return tokens;
    }
    let mut out = vec![tokens[0].clone()];
    let mut i = 1;
    while i < tokens.len() - 1 {
        if !skip(&tokens[i - 1], &tokens[i], &tokens[i + 1]) {
            out.push(tokens[i].clone());
        }
        i += 1;
    }
    out.push(tokens[i].clone());
    out
}
