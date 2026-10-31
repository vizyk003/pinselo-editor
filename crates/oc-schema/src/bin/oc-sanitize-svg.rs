//! `oc-sanitize-svg`: [`oc_schema::svg::sanitize_svg`] over a batch, for `pnpm graphics:sync` (which must not need the
//! WebAssembly engine build). Reads a JSON array of SVG strings on stdin, writes the sanitised array to stdout.

use std::io::{Read, Write};

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).expect("read stdin");
    let svgs: Vec<String> = serde_json::from_str(&input).expect("stdin: a JSON array of strings");
    let out: Vec<String> = svgs.iter().map(|s| oc_schema::svg::sanitize_svg(s)).collect();
    std::io::stdout().write_all(serde_json::to_string(&out).expect("json").as_bytes()).expect("write stdout");
}
