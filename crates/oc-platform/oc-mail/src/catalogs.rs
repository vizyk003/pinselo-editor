// The catalogs in crates/oc-platform/oc-mail/locales (embedded at compile time; `pnpm i18n:check` lists the strings).
pub(crate) const CATALOG_FILES: &[(&str, &str)] = &[
    ("ar", include_str!("../locales/ar.json")),
    ("de", include_str!("../locales/de.json")),
    ("es", include_str!("../locales/es.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("it", include_str!("../locales/it.json")),
    ("ja", include_str!("../locales/ja.json")),
    ("pt-BR", include_str!("../locales/pt-BR.json")),
    ("zh-CN", include_str!("../locales/zh-CN.json")),
];
