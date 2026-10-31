//! Port of services/api/src/print/types.ts — the fulfilment provider contract (A18): each adapter turns a paid order
//! and its print file into a fulfiller order.

use oc_schema::print::{PrintProduct, ShippingAddress};
use serde_json::{Map, Value, json};

use super::providers::Provider;

/// `FulfilmentOrder`.
#[derive(Debug, Clone)]
pub struct FulfilmentOrder {
    pub id: String,
    pub number: String,
    pub email: String,
    pub user_id: Option<String>,
    pub product_id: String,
    pub size_id: String,
    /// The order's options in their stored order.
    pub options: Vec<(String, String)>,
    pub quantity: f64,
    pub address: ShippingAddress,
    pub shipping_method: String,
    pub currency: String,
    pub shipping_cents: f64,
    pub tax_cents: f64,
    /// Orientation of the artwork (`hor` | `ver`).
    pub orientation: &'static str,
}

/// `SubmitResult`.
#[derive(Debug, Clone)]
pub struct SubmitResult {
    pub provider_order_id: Option<String>,
    pub status: String,
    pub note: Option<String>,
}

/// A status update parsed from a webhook (`ProviderUpdate`); tracking entries are `{ carrier?, code?, url? }`.
#[derive(Debug, Clone, Default)]
pub struct ProviderUpdate {
    pub event_id: String,
    /// Our order number (fulfiller external/reference id) or the fulfiller's own order id.
    pub order_number: Option<String>,
    pub provider_order_id: Option<String>,
    pub status: Option<&'static str>,
    pub tracking: Vec<Value>,
    pub note: String,
}

/// A tracking entry without its undefined members.
pub fn tracking(carrier: Option<&Value>, code: Option<&Value>, url: Option<&Value>) -> Value {
    let mut m = Map::new();
    for (k, v) in [("carrier", carrier), ("code", code), ("url", url)] {
        if let Some(v) = v {
            m.insert(k.into(), v.clone());
        }
    }
    Value::Object(m)
}

/// `splitName(name)`: last word = last name.
pub fn split_name(name: &str) -> (String, String) {
    let parts: Vec<&str> = crate::js::trim(name).split(oc_js::is_js_whitespace).filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [] => (String::new(), String::new()),
        [one] => ((*one).to_owned(), String::new()),
        [rest @ .., last] => (rest.join(" "), (*last).to_owned()),
    }
}

/// `printFileOptions(provider, p)`: the `print` options of the 'export' job that renders a provider's PDF print
/// file (`{ bleedMm, cropMarks, standard, profile, intent }`).
pub fn print_file_options(provider: Provider, p: &PrintProduct) -> Value {
    json!({
        "bleedMm": crate::http::jsjson::number_value(p.bleed_mm).unwrap_or(Value::Null),
        "cropMarks": provider.crop_marks(),
        "standard": provider.pdf_standard(),
        "profile": oc_color::registry::default_id(),
        "intent": "relative",
    })
}

#[cfg(test)]
mod tests {
    use oc_schema::print::{DEFAULT_PRINT_CATALOG, find_product};
    use serde_json::json;

    use super::super::providers::Provider;

    /// print.test.ts, 'print file options'.
    #[test]
    fn print_file_options() {
        let product = |id: &str| find_product(&DEFAULT_PRINT_CATALOG, id).expect("product");
        let bleed = |mm: f64| crate::http::jsjson::number_value(mm).expect("finite");
        let cards = product("business-cards");
        assert_eq!(super::print_file_options(Provider::Local, cards), json!({ "bleedMm": bleed(cards.bleed_mm), "cropMarks": true, "standard": "pdfx4", "profile": "pso-coated-v3", "intent": "relative" }));
        let mugs = product("mugs");
        assert_eq!(
            super::print_file_options(Provider::Gelato, mugs),
            json!({ "bleedMm": bleed(mugs.bleed_mm), "cropMarks": false, "standard": "pdfx4", "profile": oc_color::registry::default_id(), "intent": "relative" })
        );
        assert_eq!(Provider::Gelato.file_format(), "pdf");
        assert_eq!(Provider::Local.file_format(), "pdf");
        // Printful gets PNGs: no PDF standard.
        assert!(product("tote-bags").fulfilment.as_ref().is_some_and(|f| f.printful.is_some()));
        assert_eq!(Provider::Printful.file_format(), "png");
        assert_eq!(Provider::Printful.pdf_standard(), "none");
        assert!(!Provider::Printful.crop_marks());
    }

    #[test]
    fn names() {
        assert_eq!(super::split_name("  Ada  Lovelace King "), ("Ada Lovelace".into(), "King".into()));
        assert_eq!(super::split_name("Cher"), ("Cher".into(), String::new()));
        assert_eq!(super::split_name(""), (String::new(), String::new()));
    }
}
