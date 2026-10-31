// SPDX-License-Identifier: Apache-2.0
//! Structural parity with the TS (SkParagraph) layout: tests/golden/text/*.json, written by
//! `pnpm tsx tools/golden/text.ts`.
//!
//! Per case the outcome is:
//!
//! - **exact**: same case-transformed text, same line count and identical line break positions (UTF-16 start and
//!   end-excluding-whitespace of every line) in every paragraph, identical paragraph and total heights
//!   (|Δ| < 0.01 px), identical list markers and shrink-to-fit scale, curved box within 0.1%;
//! - **tolerant**: same text and line count per paragraph, total height within [`HEIGHT_TOL`] (relative) and
//!   curved box within [`CURVE_TOL`], but some break position or height differs;
//! - **failing**: anything else.
//!
//! For cases with the same line count the `LineMetrics` are compared too ([`MetricStats`]): `left`, `baseline`,
//! `ascent` and `descent` must match to 0.01 px; `width`, `getLongestLine` and `getMaxIntrinsicWidth` may differ by
//! sub-pixel amounts (`f32` summation order; parley resolves the script of a space at a bidi boundary from the
//! preceding text where SkShaper uses the following text, which changes kerning). Glyph x positions are compared
//! for the hand cases ([`glyph_positions`]); ADR-0001 accepts differences there, but HarfRust matches
//! SkParagraph's HarfBuzz to 0.01 px on almost every line.

mod common;

use oc_schema::text::TextBody;
use oc_schema::text::TextEffect;
use serde_json::Value;

/// Relative total-height tolerance for "tolerant" cases.
const HEIGHT_TOL: f64 = 0.02;
/// Relative tolerance for curved-text box sizes (they depend on glyph advances).
const CURVE_TOL: f64 = 0.02;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Outcome {
    Exact,
    Tolerant,
    Failing,
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1.0)
}

/// `LineMetrics` comparison over every line of every case with the same line count.
#[derive(Debug, Default)]
struct MetricStats {
    lines: usize,
    paras: usize,
    /// `left`, `baseline`, `ascent`, `descent` off by more than 0.01 px.
    exact_misses: Vec<String>,
    width_off: usize,
    width_max: f64,
    longest_off: usize,
    longest_max: f64,
    intrinsic_off: usize,
    intrinsic_max: f64,
}

/// Rounding to 0.01 px (`littleRound`) on both sides can differ by one step.
const ROUNDED_TOL: f64 = 0.0105;

fn compare_metrics(name: &str, p: &oc_text::LaidParagraph, w: &Value, m: &mut MetricStats) {
    m.paras += 1;
    let longest = f(&w["longestLine"]);
    // An empty paragraph's longest line is `-FLT_MAX` on both sides.
    let dl = if longest < -1e38 && p.longest_line < -1e38 { 0.0 } else { (p.longest_line - longest).abs() };
    m.longest_off += usize::from(dl > ROUNDED_TOL);
    m.longest_max = m.longest_max.max(dl);
    let di = (p.max_intrinsic_width - f(&w["maxIntrinsicWidth"])).abs();
    m.intrinsic_off += usize::from(di > ROUNDED_TOL);
    m.intrinsic_max = m.intrinsic_max.max(di);
    for (j, (l, wl)) in p.lines.iter().zip(w["lines"].as_array().unwrap()).enumerate() {
        m.lines += 1;
        for (what, got, want) in [("left", l.left, &wl["left"]), ("baseline", l.baseline, &wl["baseline"]), ("ascent", l.ascent, &wl["ascent"]), ("descent", l.descent, &wl["descent"])] {
            if (got - f(want)).abs() > 0.01 {
                m.exact_misses.push(format!("{name} l{j} {what} {got} vs {want}"));
            }
        }
        let dw = (l.width - f(&wl["width"])).abs();
        m.width_off += usize::from(dw > ROUNDED_TOL);
        m.width_max = m.width_max.max(dw);
    }
}

fn run_case(engine: &mut oc_text::TextEngine, case: &Value, metrics: &mut MetricStats) -> (Outcome, String) {
    let body: TextBody = serde_json::from_value(case["body"].clone()).unwrap();
    let width = f(&case["width"]);
    let effect: Option<TextEffect> = case.get("effect").map(|e| serde_json::from_value(e.clone()).unwrap());
    common::ensure(engine, &body);
    let want = &case["out"];
    let key = case["name"].as_str().unwrap();

    if let Some(curve) = case.get("curve").and_then(Value::as_f64) {
        let got = engine.curved(key, &body, curve);
        let w = &want["curved"];
        return match (got, w.is_null()) {
            (None, true) => (Outcome::Exact, String::new()),
            (Some(g), false) => {
                let dw = rel(g.width, f(&w["width"]));
                let dh = rel(g.height, f(&w["height"]));
                let same_n = g.glyphs.len() as u64 == w["glyphs"].as_u64().unwrap();
                let msg = format!("curved {}x{} n={} vs {}x{} n={}", g.width, g.height, g.glyphs.len(), w["width"], w["height"], w["glyphs"]);
                if same_n && dw < 1e-3 && dh < 1e-3 {
                    (Outcome::Exact, msg)
                } else if same_n && dw < CURVE_TOL && dh < CURVE_TOL {
                    (Outcome::Tolerant, msg)
                } else {
                    (Outcome::Failing, msg)
                }
            }
            _ => (Outcome::Failing, "curved presence differs".into()),
        };
    }

    let (layout, scale) = match case.get("fitHeight").and_then(Value::as_f64) {
        Some(h) => {
            let (l, s) = engine.layout_fit(key, &body, width, h, effect.as_ref());
            (l, Some(s))
        }
        None => (engine.layout(key, &body, width, effect.as_ref()), None),
    };
    let mut exact = true;
    let mut notes: Vec<String> = Vec::new();
    if let Some(s) = scale {
        let ws = f(&want["scale"]);
        if (s - ws).abs() > 1e-9 {
            exact = false;
            notes.push(format!("scale {s} vs {ws}"));
        }
    }
    let wparas = want["paras"].as_array().unwrap();
    if wparas.len() != layout.paras.len() {
        return (Outcome::Failing, format!("paragraph count {} vs {}", layout.paras.len(), wparas.len()));
    }
    let mut same_counts = true;
    for (i, (p, w)) in layout.paras.iter().zip(wparas).enumerate() {
        if p.text != w["text"].as_str().unwrap() {
            return (Outcome::Failing, format!("p{i} text {:?} vs {:?}", p.text, w["text"]));
        }
        let wl = w["lines"].as_array().unwrap();
        if wl.len() != p.lines.len() {
            same_counts = false;
            notes.push(format!("p{i} lines {} vs {}", p.lines.len(), wl.len()));
        } else {
            compare_metrics(key, p, w, metrics);
            for (j, (l, wl)) in p.lines.iter().zip(wl).enumerate() {
                let (ws, we) = (wl["start"].as_u64().unwrap() as usize, wl["endNoWs"].as_u64().unwrap() as usize);
                if l.start != ws || l.end_no_ws != we {
                    exact = false;
                    notes.push(format!("p{i} l{j} [{}, {}) vs [{ws}, {we})", l.start, l.end_no_ws));
                }
            }
        }
        if (p.height - f(&w["height"])).abs() > 0.01 {
            exact = false;
            notes.push(format!("p{i} height {} vs {}", p.height, w["height"]));
        }
        if (p.y - f(&w["y"])).abs() > 0.01 || (p.x - f(&w["x"])).abs() > 1e-6 || p.start as u64 != w["start"].as_u64().unwrap() || p.len as u64 != w["len"].as_u64().unwrap() {
            exact = false;
            notes.push(format!("p{i} box x{} y{} s{} n{} vs x{} y{} s{} n{}", p.x, p.y, p.start, p.len, w["x"], w["y"], w["start"], w["len"]));
        }
        match (&p.bullet, w.get("bullet")) {
            (None, None) => {}
            (Some(b), Some(wb)) => {
                if (b.width - f(&wb["width"])).abs() > 1e-6 || (b.height - f(&wb["height"])).abs() > 0.01 {
                    exact = false;
                    notes.push(format!("p{i} bullet {}x{} vs {}x{}", b.width, b.height, wb["width"], wb["height"]));
                }
            }
            _ => return (Outcome::Failing, format!("p{i} bullet presence")),
        }
    }
    let wh = f(&want["height"]);
    if (layout.height - wh).abs() > 0.01 {
        exact = false;
        notes.push(format!("height {} vs {wh}", layout.height));
    }
    if layout.length as u64 != want["length"].as_u64().unwrap() {
        return (Outcome::Failing, format!("length {} vs {}", layout.length, want["length"]));
    }
    let msg = notes.join("; ");
    if same_counts && exact {
        (Outcome::Exact, msg)
    } else if same_counts && rel(layout.height, wh) <= HEIGHT_TOL {
        (Outcome::Tolerant, msg)
    } else {
        (Outcome::Failing, msg)
    }
}

struct Stats {
    exact: usize,
    tolerant: Vec<(String, String)>,
    failing: Vec<(String, String)>,
    metrics: MetricStats,
}

impl Stats {
    fn total(&self) -> usize {
        self.exact + self.tolerant.len() + self.failing.len()
    }

    /// The assertions shared by every golden file.
    fn check(&self, name: &str) {
        let total = self.total();
        assert!(self.failing.is_empty(), "{} of {total} {name} cases failing", self.failing.len());
        assert!(self.exact * 100 >= total * 99, "only {} of {total} {name} cases exact", self.exact);
        let m = &self.metrics;
        assert!(m.exact_misses.is_empty(), "{name}: line metrics differ: {:?}", &m.exact_misses[..m.exact_misses.len().min(20)]);
        // Sub-pixel differences are allowed on at most 1% of the lines (2% of the paragraphs for the unbroken width,
        // a sum over the whole paragraph), and never more than half a pixel.
        let few = |n: usize, of: usize, pct: usize| n <= (of * pct / 100).max(1);
        assert!(few(m.width_off, m.lines, 1) && m.width_max <= 0.5, "{name}: {} of {} line widths off (max {})", m.width_off, m.lines, m.width_max);
        assert!(few(m.longest_off, m.paras, 1) && m.longest_max <= 0.5, "{name}: {} of {} longest lines off (max {})", m.longest_off, m.paras, m.longest_max);
        assert!(few(m.intrinsic_off, m.paras, 2) && m.intrinsic_max <= 0.5, "{name}: {} of {} intrinsic widths off (max {})", m.intrinsic_off, m.paras, m.intrinsic_max);
    }
}

fn run_file(name: &str) -> Stats {
    let cases = common::golden(name);
    let mut engine = common::engine();
    let mut st = Stats { exact: 0, tolerant: Vec::new(), failing: Vec::new(), metrics: MetricStats::default() };
    for case in cases.as_array().unwrap() {
        let (o, msg) = run_case(&mut engine, case, &mut st.metrics);
        let n = case["name"].as_str().unwrap().to_owned();
        match o {
            Outcome::Exact => st.exact += 1,
            Outcome::Tolerant => st.tolerant.push((n, msg)),
            Outcome::Failing => st.failing.push((n, msg)),
        }
    }
    let m = &st.metrics;
    eprintln!(
        "{name}: {} cases — exact {}, tolerant {}, failing {}; {} lines: width off {} (max {:.3}), longest off {} (max {:.3}), intrinsic off {} (max {:.3})",
        st.total(),
        st.exact,
        st.tolerant.len(),
        st.failing.len(),
        m.lines,
        m.width_off,
        m.width_max,
        m.longest_off,
        m.longest_max,
        m.intrinsic_off,
        m.intrinsic_max
    );
    for (n, m) in st.failing.iter().take(40) {
        eprintln!("  FAIL {n}: {m}");
    }
    for (n, m) in st.tolerant.iter().take(40) {
        eprintln!("  tol  {n}: {m}");
    }
    st
}

#[test]
fn hand_cases() {
    run_file("hand.json").check("hand");
}

#[test]
fn template_cases() {
    run_file("templates.json").check("template");
}

#[test]
fn stress_cases() {
    run_file("stress.json").check("stress");
}

/// RTL, CJK, Thai, Korean, emoji and fallback text. Thai breaks like ICU4C since complex-context runs end by the
/// UAX #14 rules (see `layout::break_opportunities`); unresolved emoji split SkParagraph's runs as they do in TS.
#[test]
fn intl_cases() {
    run_file("intl.json").check("intl");
}

/// Glyph x positions (`getShapedLines`, painted glyphs, trailing whitespace excluded) of every hand case line.
/// ADR-0001 accepts differences here, but they are a regression signal: every line must have the same glyphs,
/// at most 5% of the lines may be off by more than 0.01 px and none by more than a pixel (see the module docs).
#[test]
fn glyph_positions() {
    let cases = common::golden("hand.json");
    let mut engine = common::engine();
    let (mut lines, mut off, mut worst) = (0usize, Vec::new(), 0.0f64);
    for case in cases.as_array().unwrap() {
        if case.get("curve").is_some() {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let mut body: TextBody = serde_json::from_value(case["body"].clone()).unwrap();
        if let Some(s) = case["out"]["scale"].as_f64().filter(|s| *s != 1.0) {
            body = oc_text::scale_body(&body, s);
        }
        common::ensure(&mut engine, &body);
        let l = engine.layout_uncached(&body, f(&case["width"]), None);
        for (p, w) in l.paras.iter().zip(case["out"]["paras"].as_array().unwrap()) {
            let Some(gx) = w.get("glyphX").and_then(Value::as_array) else { continue };
            assert_eq!(p.lines.len(), gx.len(), "{name} line count");
            for (j, (line, want)) in p.lines.iter().zip(gx).enumerate() {
                lines += 1;
                let mut got: Vec<f64> = line.glyph_runs.iter().flat_map(|r| r.glyphs.iter().map(|g| g.x)).collect();
                let mut want: Vec<f64> = want.as_array().unwrap().iter().map(f).collect();
                got.sort_by(f64::total_cmp);
                want.sort_by(f64::total_cmp);
                assert_eq!(got.len(), want.len(), "{name} l{j} glyph count");
                let d = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
                worst = worst.max(d);
                if d > 0.01 {
                    off.push(format!("{name} l{j} {d:.3}"));
                }
            }
        }
    }
    eprintln!("glyph positions: {lines} lines, {} off by more than 0.01 px (worst {worst:.3}): {off:?}", off.len());
    assert!(lines > 100);
    assert!(off.len() * 20 <= lines && worst <= 1.0, "{} of {lines} lines off, worst {worst}", off.len());
}

/// Caret and selection geometry against the TS (`caretRect`, `wordAt`, `verticalMove`, `offsetAt`,
/// `selectionRects`). Offsets must match exactly; x coordinates within [`CARET_X_TOL`] px (glyph advances
/// differ sub-pixel); y/heights exactly. At most 5% of the `offsetAt` probes and `verticalMove` results may differ:
/// probes near a cluster edge may resolve to the neighbour, and three SkParagraph hit-testing quirks are not
/// reproduced (`getGlyphPositionAtCoordinate` returns the offset *before* the line for a point right of a line
/// that ends with an RTL run, an offset inside a grapheme of several `.notdef` glyphs, and the line end for a
/// point exactly on the right edge of a justified line).
const CARET_X_TOL: f64 = 1.0;

#[test]
fn caret_geometry() {
    let cases = common::golden("hand.json");
    let mut engine = common::engine();
    let mut checked = 0;
    let (mut probes, mut probe_diff) = (0usize, 0usize);
    let (mut vtotal, mut vdiff) = (0usize, 0usize);
    for case in cases.as_array().unwrap() {
        let Some(want) = case["out"].get("caret") else { continue };
        let body: TextBody = serde_json::from_value(case["body"].clone()).unwrap();
        common::ensure(&mut engine, &body);
        let l = engine.layout(case["name"].as_str().unwrap(), &body, f(&case["width"]), None);
        let name = case["name"].as_str().unwrap();
        for (o, w) in want["caretRect"].as_array().unwrap().iter().enumerate() {
            let r = l.caret_rect(o);
            assert!((r.x - f(&w["x"])).abs() <= CARET_X_TOL, "{name} caretRect({o}).x {} vs {}", r.x, w["x"]);
            assert!((r.y - f(&w["y"])).abs() < 0.01 && (r.h - f(&w["h"])).abs() < 0.01, "{name} caretRect({o}) {r:?} vs {w}");
        }
        for (o, w) in want["wordAt"].as_array().unwrap().iter().enumerate() {
            let (a, b) = l.word_at(o);
            assert_eq!([a as u64, b as u64], [w[0].as_u64().unwrap(), w[1].as_u64().unwrap()], "{name} wordAt({o})");
        }
        for (o, w) in want["verticalDown"].as_array().unwrap().iter().enumerate() {
            let got = l.vertical_move(o, 1) as u64;
            let wv = w.as_u64().unwrap();
            if got != wv {
                eprintln!("  {name} verticalMove({o}, 1) {got} vs {wv}");
                vdiff += 1;
            }
            vtotal += 1;
        }
        for (o, w) in want["verticalUp"].as_array().unwrap().iter().enumerate() {
            let got = l.vertical_move(o, -1) as u64;
            let wv = w.as_u64().unwrap();
            if got != wv {
                eprintln!("  {name} verticalMove({o}, -1) {got} vs {wv}");
                vdiff += 1;
            }
            vtotal += 1;
        }
        for p in want["offsetAt"].as_array().unwrap() {
            probes += 1;
            let got = l.offset_at(f(&p["x"]), f(&p["y"])) as u64;
            if got != p["out"].as_u64().unwrap() {
                eprintln!("  {name} offsetAt({}, {}) {got} vs {}", p["x"], p["y"], p["out"]);
                probe_diff += 1;
            }
        }
        let sel = &want["selection"];
        let rects = l.selection_rects(sel["from"].as_u64().unwrap() as usize, sel["to"].as_u64().unwrap() as usize);
        let wr = sel["rects"].as_array().unwrap();
        assert_eq!(rects.len(), wr.len(), "{name} selectionRects count {rects:?} vs {wr:?}");
        for (r, w) in rects.iter().zip(wr) {
            assert!((r.x - f(&w["x"])).abs() <= CARET_X_TOL && (r.w - f(&w["w"])).abs() <= CARET_X_TOL, "{name} selection {r:?} vs {w}");
            assert!((r.y - f(&w["y"])).abs() < 0.01 && (r.h - f(&w["h"])).abs() < 0.01, "{name} selection {r:?} vs {w}");
        }
        checked += 1;
    }
    eprintln!("caret: {checked} cases, offsetAt probes {probes} ({probe_diff} differ), verticalMove {vtotal} ({vdiff} differ)");
    assert!(vdiff * 20 <= vtotal, "{vdiff} of {vtotal} verticalMove results differ");
    assert!(checked >= 10);
    assert!(probe_diff * 20 <= probes, "{probe_diff} of {probes} offsetAt probes differ");
}
