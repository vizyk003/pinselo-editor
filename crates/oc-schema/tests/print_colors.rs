// SPDX-License-Identifier: Apache-2.0
//! `print_colors` / `print_color_key` and the `out_of_gamut` override of `preflight_print_with` over the seed
//! templates (the TS parity of the same functions runs in packages/engine/test/helpers-goldens.test.ts).

mod common;

use std::collections::HashSet;

use common::*;
use oc_schema::DesignDoc;
use oc_schema::primitives::ColorValue;
use oc_schema::print::{cmyk_gamut_risk, preflight_print, preflight_print_with, print_color_key, print_colors, standalone_print_target};

fn gamut_issues(r: &oc_schema::print::PreflightResult) -> usize {
    r.issues.iter().filter(|i| i.code == "out_of_gamut").count()
}

#[test]
fn print_colors_and_gamut_override() {
    let (mut colours, mut risky) = (0, 0);
    for file in &fixture_files() {
        let doc: DesignDoc = serde_json::from_value(fixture(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let list = print_colors(&doc, 512);
        let keys: HashSet<String> = list.iter().map(print_color_key).collect();
        assert_eq!(keys.len(), list.len(), "{file}: duplicate colours");
        assert!(list.iter().all(|c| !matches!(c, ColorValue::Cmyk(_) | ColorValue::Spot(_))), "{file}: CMYK/spot listed");
        assert_eq!(print_colors(&doc, 1).len(), list.len().min(1), "{file}: cap");
        colours += list.len();
        risky += list.iter().filter(|c| cmyk_gamut_risk(c)).count();

        let target = standalone_print_target(&doc);
        let plain = preflight_print(&doc, &target, None);
        let fallback = |_: &ColorValue| None;
        assert_eq!(j(&preflight_print_with(&doc, &target, None, Some(&fallback))), j(&plain), "{file}: undefined falls back");
        let never = |_: &ColorValue| Some(false);
        assert_eq!(gamut_issues(&preflight_print_with(&doc, &target, None, Some(&never))), 0, "{file}");
        let always = |_: &ColorValue| Some(true);
        assert!(gamut_issues(&preflight_print_with(&doc, &target, None, Some(&always))) >= gamut_issues(&plain), "{file}");
    }
    assert!(colours > 100 && risky > 0, "{colours} colours, {risky} risky");
}

#[test]
fn print_color_key_format() {
    let c = ColorValue::srgb(0.5, 1.0, 0.0, 0.3);
    assert_eq!(print_color_key(&c), "srgb:500,1000,0");
}
