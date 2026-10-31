// SPDX-License-Identifier: Apache-2.0
//! The bundled print profiles (`crates/oc-color/icc-profiles.json`, shared with `pnpm icc:sync` and the API).

use std::path::Path;
use std::sync::OnceLock;

use serde::Deserialize;

const JSON: &str = include_str!("../icc-profiles.json");
/// The compact sRGB v4 profile (CC0), shipped in-repo.
pub const SRGB_ICC: &[u8] = include_bytes!("../icc/sRGB-v4.icc");

/// One bundled profile and the metadata a PDF/X output intent needs.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BundledProfile {
    pub id: String,
    pub name: String,
    /// `CMYK` or `RGB`.
    pub color_space: String,
    /// `/OutputConditionIdentifier` (characterised printing condition in the ICC registry), e.g. `FOGRA51`.
    pub condition_id: String,
    /// `/OutputCondition`.
    pub condition: String,
    pub description: String,
    /// `coated`, `uncoated` or `none`.
    pub paper: String,
    /// File name under `fixtures/icc/` (or the in-repo sRGB).
    pub file: String,
    /// sha256 of the `.icc` file.
    pub sha256: String,
    #[serde(default)]
    pub builtin: bool,
}

#[derive(Debug, Deserialize)]
struct Registry {
    registry: String,
    default: String,
    profiles: Vec<BundledProfile>,
}

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| serde_json::from_str(JSON).expect("icc-profiles.json is valid"))
}

/// The ICC registry name used in output intents (`http://www.color.org`).
pub fn registry_name() -> &'static str {
    &registry().registry
}

/// All bundled profiles, in display order.
pub fn bundled() -> &'static [BundledProfile] {
    &registry().profiles
}

/// The default print profile id (`pso-coated-v3`).
pub fn default_id() -> &'static str {
    &registry().default
}

pub fn find(id: &str) -> Option<&'static BundledProfile> {
    bundled().iter().find(|p| p.id == id)
}

/// Read a bundled profile's bytes: the builtin sRGB from the binary, the others from `dir` (`fixtures/icc`).
pub fn read_bundled(id: &str, dir: &Path) -> Result<Vec<u8>, crate::ColorError> {
    let p = find(id).ok_or_else(|| crate::ColorError::UnknownProfile(id.to_string()))?;
    if p.builtin {
        return Ok(SRGB_ICC.to_vec());
    }
    std::fs::read(dir.join(&p.file))
        .map_err(|e| crate::ColorError::Missing(format!("{} ({}): {e}; run `pnpm icc:sync`", p.name, dir.join(&p.file).display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_parses_and_default_exists() {
        assert_eq!(registry_name(), "http://www.color.org");
        let d = find(default_id()).expect("default profile");
        assert_eq!(d.condition_id, "FOGRA51");
        assert!(bundled().iter().any(|p| p.id == "srgb" && p.builtin));
        assert_eq!(find("iso-coated-v2").map(|p| p.condition_id.as_str()), Some("FOGRA39"));
        assert_eq!(find("pso-uncoated-v3").map(|p| p.condition_id.as_str()), Some("FOGRA52"));
    }
}
