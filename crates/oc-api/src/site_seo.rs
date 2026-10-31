//! Port of services/api/src/siteSeo.ts — robots.txt and sitemap.xml for published websites (WS-011).

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// `robotsTxt({ noindex, sitemapUrl })`: hidden (noindex) or password-protected sites ask crawlers to stay out;
/// others point at their sitemap.
pub fn robots_txt(noindex: bool, sitemap_url: Option<&str>) -> String {
    if noindex {
        return "User-agent: *\nDisallow: /\n".into();
    }
    let sitemap = sitemap_url.filter(|u| !u.is_empty()).map(|u| format!("\nSitemap: {u}\n")).unwrap_or_default();
    format!("User-agent: *\nAllow: /\n{sitemap}")
}

/// `sitemapXml(entries)`: `lastmod` is an ISO string (its date part is used) or `None`.
pub fn sitemap_xml(entries: &[(String, Option<String>)]) -> String {
    let urls: Vec<String> = entries
        .iter()
        .map(|(loc, lastmod)| {
            let lm = lastmod.as_deref().map(|l| crate::js::slice(l, 0, 10)).unwrap_or_default();
            format!("  <url><loc>{}</loc>{}</url>", xml(loc), if lm.is_empty() { String::new() } else { format!("<lastmod>{lm}</lastmod>") })
        })
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n{}{}</urlset>\n",
        urls.join("\n"),
        if urls.is_empty() { "" } else { "\n" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seo_files() {
        assert_eq!(robots_txt(true, Some("x")), "User-agent: *\nDisallow: /\n");
        assert_eq!(robots_txt(false, Some("https://a/s.xml")), "User-agent: *\nAllow: /\n\nSitemap: https://a/s.xml\n");
        assert_eq!(robots_txt(false, None), "User-agent: *\nAllow: /\n");
        assert_eq!(
            sitemap_xml(&[("https://a/?x=1&y=<2>".into(), Some("2025-01-02T03:04:05.000Z".into()))]),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n  <url><loc>https://a/?x=1&amp;y=&lt;2&gt;</loc><lastmod>2025-01-02</lastmod></url>\n</urlset>\n"
        );
        assert_eq!(sitemap_xml(&[]), "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n</urlset>\n");
    }
}
