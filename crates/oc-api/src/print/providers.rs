//! Port of services/api/src/print/providers.ts — fulfiller selection: `OC_PRINT_PROVIDER` = auto (default) |
//! gelato | printful | local.

use oc_schema::print::PrintProduct;

use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Gelato,
    Printful,
    Local,
}

pub const PROVIDERS: [Provider; 3] = [Provider::Gelato, Provider::Printful, Provider::Local];

impl Provider {
    pub fn id(self) -> &'static str {
        match self {
            Provider::Gelato => "gelato",
            Provider::Printful => "printful",
            Provider::Local => "local",
        }
    }

    pub fn parse(id: &str) -> Option<Provider> {
        PROVIDERS.into_iter().find(|p| p.id() == id)
    }

    /// Configured (API key present) — the local provider always is.
    pub fn enabled(self, s: &AppState) -> bool {
        match self {
            Provider::Gelato => s.env_nonempty("OC_GELATO_API_KEY").is_some(),
            Provider::Printful => s.env_nonempty("OC_PRINTFUL_API_KEY").is_some(),
            Provider::Local => true,
        }
    }

    /// The product is mapped for this fulfiller.
    pub fn supports(self, p: &PrintProduct) -> bool {
        let f = p.fulfilment.as_ref();
        match self {
            Provider::Gelato => f.is_some_and(|f| f.gelato.is_some()),
            Provider::Printful => f.is_some_and(|f| f.printful.is_some()),
            Provider::Local => true,
        }
    }

    /// File the fulfiller wants: a print PDF with bleed, or a PNG of the print area at 300 dpi.
    pub fn file_format(self) -> &'static str {
        if self == Provider::Printful { "png" } else { "pdf" }
    }

    /// Crop marks in the PDF (a human cuts it); fulfillers want bleed only.
    pub fn crop_marks(self) -> bool {
        self == Provider::Local
    }

    /// PDF standard of the print file (PDF files only): PDF/X-4 with the default CMYK output intent; Printful
    /// gets PNGs.
    pub fn pdf_standard(self) -> &'static str {
        if self == Provider::Printful { "none" } else { "pdfx4" }
    }
}

/// `providerFor(p, pref)`.
pub fn provider_for(s: &AppState, p: &PrintProduct) -> Provider {
    let pref = s.env("OC_PRINT_PROVIDER").unwrap_or_else(|| "auto".into());
    if pref == "local" {
        return Provider::Local;
    }
    let order = if pref == "printful" { [Provider::Printful, Provider::Gelato] } else { [Provider::Gelato, Provider::Printful] };
    if pref != "auto" && pref != "gelato" && pref != "printful" {
        return Provider::Local;
    }
    order.into_iter().find(|f| f.enabled(s) && f.supports(p)).unwrap_or(Provider::Local)
}

/// `enabledProviders()`.
pub fn enabled_providers(s: &AppState) -> Vec<&'static str> {
    PROVIDERS.into_iter().filter(|p| p.enabled(s)).map(Provider::id).collect()
}
