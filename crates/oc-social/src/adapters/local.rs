//! Built-in default: "Download & remind". At the scheduled time the worker renders the export and sends an
//! email + in-app notification with the file link and the caption to copy; no external account needed.

use crate::http::Http;
use crate::types::{
    AdapterFuture, AuthKind, Limits, PublishContext, PublishResult, SocialAdapter,
};

pub struct Local;
pub static LOCAL: Local = Local;

static LIMITS: Limits = Limits {
    max_chars: 100_000,
    images: 100,
    video: true,
    image_types: &["image/png", "image/jpeg"],
    max_image_bytes: None,
};

impl SocialAdapter for Local {
    fn id(&self) -> &'static str {
        "local"
    }
    fn label(&self) -> &'static str {
        "Download & remind"
    }
    fn auth(&self) -> AuthKind {
        AuthKind::None
    }
    fn env(&self) -> &'static [&'static str] {
        &[]
    }
    fn configured(&self, _http: &Http) -> bool {
        true
    }
    fn limits(&self) -> &Limits {
        &LIMITS
    }
    fn publish<'a>(
        &'a self,
        _http: &'a Http,
        ctx: &'a PublishContext,
    ) -> AdapterFuture<'a, PublishResult> {
        Box::pin(async move {
            Ok(PublishResult {
                url: ctx.media.first().map(|m| m.url.clone()),
                external_id: None,
                note: Some("Reminder sent with the files and caption.".into()),
            })
        })
    }
}
