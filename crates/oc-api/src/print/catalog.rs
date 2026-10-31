//! Port of services/api/src/print/catalog.ts — the print catalogue served to the editor and used for quotes.
//! Operators replace it with JSON (`OC_PRINT_CATALOG` = a file path or inline JSON, same shape as
//! `DEFAULT_PRINT_CATALOG`). Prices are minor units of the catalogue currency, or of `OC_BILLING_CURRENCY` when the
//! catalogue has none.

use std::sync::{Arc, Mutex};

use oc_schema::print::{DEFAULT_PRINT_CATALOG, PrintCatalog, check_print_catalog};

use crate::http::error::ApiError;
use crate::state::AppState;

#[derive(Default)]
struct Cache(Mutex<Option<(String, Arc<PrintCatalog>)>>);

/// `printCatalog()` (cached per `OC_PRINT_CATALOG` value; a broken catalogue is an error, as the TS throws).
pub fn print_catalog(s: &AppState) -> Result<Arc<PrintCatalog>, ApiError> {
    let src = s.env("OC_PRINT_CATALOG").unwrap_or_default();
    let cache = s.local::<Cache>();
    if let Some((key, c)) = cache.0.lock().expect("catalog").as_ref()
        && *key == src
    {
        return Ok(c.clone());
    }
    let mut c: PrintCatalog = if src.is_empty() {
        DEFAULT_PRINT_CATALOG.clone()
    } else {
        let json = if src.trim().starts_with('{') { src.clone() } else { std::fs::read_to_string(&src)? };
        let c: PrintCatalog = serde_json::from_str(&json)?;
        check_print_catalog(&c).map_err(ApiError::internal)?;
        c
    };
    c.currency = if c.currency.is_empty() { crate::billing::billing_currency(s) } else { c.currency.to_lowercase() };
    let c = Arc::new(c);
    *cache.0.lock().expect("catalog") = Some((src, c.clone()));
    Ok(c)
}
