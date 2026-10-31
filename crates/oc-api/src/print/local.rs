//! Port of services/api/src/print/local.ts — the local "print-ready file" provider (the default): the order's PDF
//! (bleed + crop marks) is stored and the operator (`OC_PRINT_OPERATOR_EMAIL`) gets an email with the file link and
//! the order details to print it themselves or forward it to any print shop. Staff move the order along in the
//! admin console.

use oc_schema::print::PrintProduct;

use crate::http::error::ApiError;
use crate::print::types::{FulfilmentOrder, SubmitResult};
use crate::state::AppState;

pub fn operator_email(s: &AppState) -> Option<String> {
    s.env("OC_PRINT_OPERATOR_EMAIL").map(|e| crate::js::trim(&e).to_owned()).filter(|e| !e.is_empty())
}

/// `localPrint.submit(order, product, fileUrl)`.
pub async fn submit(s: &AppState, o: &FulfilmentOrder, p: &PrintProduct, file_url: &str) -> Result<SubmitResult, ApiError> {
    let to = operator_email(s);
    let a = &o.address;
    let options: Vec<String> = o
        .options
        .iter()
        .map(|(k, v)| {
            let g = p.options.iter().find(|g| &g.id == k);
            let label = g.map_or(k.as_str(), |g| g.label.as_str());
            let choice = g.and_then(|g| g.choices.iter().find(|c| &c.id == v)).map_or(v.as_str(), |c| c.label.as_str());
            format!("{label}: {choice}")
        })
        .collect();
    let options = options.join(", ");
    if let Some(to) = &to {
        let size = p.sizes.iter().find(|x| x.id == o.size_id).map_or(o.size_id.as_str(), |x| x.label.as_str());
        let city = format!("{} {}{}", a.postal_code, a.city, a.state.as_deref().map(|st| format!(", {st}")).unwrap_or_default());
        let email = a.email.clone().unwrap_or_else(|| o.email.clone());
        let lines: Vec<&str> =
            [Some(a.name.as_str()), a.company.as_deref(), Some(a.line1.as_str()), a.line2.as_deref(), Some(city.as_str()), Some(a.country.as_str()), a.phone.as_deref(), Some(email.as_str())]
                .into_iter()
                .flatten()
                .filter(|l| !l.is_empty())
                .collect();
        let tpl = oc_mail::Template::PrintOperator {
            number: o.number.clone(),
            product: format!("{} — {size}{}", p.name, if options.is_empty() { String::new() } else { format!(" ({options})") }),
            quantity: oc_js::number_to_string(o.quantity),
            shipping: o.shipping_method.clone(),
            address: lines.join("\n"),
            file_url: file_url.to_owned(),
            url: format!("{}/admin/print", s.cfg.web_origin),
        };
        oc_mail::send_email(&s.pool, to, &tpl, None).await?;
    }
    Ok(SubmitResult {
        provider_order_id: None,
        status: "submitted".into(),
        note: Some(match to {
            Some(t) => format!("Print file sent to {t}."),
            None => "Print file ready (no OC_PRINT_OPERATOR_EMAIL set).".into(),
        }),
    })
}
