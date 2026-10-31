//! `await req.file({ limits: { fileSize } })` of `@fastify/multipart` 10 (registered with `limits: { files: 1 }`):
//! the first file part of a `multipart/form-data` body, streamed to disk.
//!
//! * a request that is not multipart → 406 `the request is not multipart`;
//! * no file part → `None` (the route answers `400 no_file`);
//! * a file over `fileSize` is cut off there and flagged `truncated` (busboy's `limit` event; the route then
//!   answers its own 413 — the transcripts show the `truncated` path is what Fastify takes here).

use std::path::Path;

use tokio::io::AsyncWriteExt as _;

use crate::http::error::ApiError;
use crate::http::req::{Body, Req};

pub struct UploadedFile {
    pub filename: String,
    pub mimetype: String,
    pub fieldname: String,
    pub size: u64,
    /// More than `fileSize` bytes were sent (`file.file.truncated`).
    pub truncated: bool,
}

/// Stream the first file of the request into `dest`.
pub async fn save_file(req: &mut Req, file_size_limit: u64, dest: &Path) -> Result<Option<UploadedFile>, ApiError> {
    let Body::Multipart { content_type, body } = std::mem::replace(&mut req.body, Body::None) else {
        return Err(ApiError::status(406, "the request is not multipart"));
    };
    let Some(body) = body.into_inner().ok().flatten() else {
        return Err(ApiError::status(406, "the request is not multipart"));
    };
    let boundary = multer::parse_boundary(&content_type).map_err(|_| ApiError::status(406, "the request is not multipart"))?;
    let stream = body.into_data_stream();
    let mut mp = multer::Multipart::new(stream, boundary);
    loop {
        let field = match mp.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => return Ok(None),
            Err(e) => return Err(ApiError::status(400, e.to_string())),
        };
        let Some(filename) = field.file_name().map(str::to_owned) else { continue };
        let fieldname = field.name().unwrap_or("").to_owned();
        let mimetype = field.content_type().map_or_else(|| "application/octet-stream".to_owned(), |m| m.to_string());
        let mut out = tokio::fs::File::create(dest).await?;
        let mut size: u64 = 0;
        let mut truncated = false;
        let mut field = field;
        while let Some(chunk) = field.chunk().await.map_err(|e| ApiError::status(400, e.to_string()))? {
            if truncated {
                continue;
            }
            let room = file_size_limit.saturating_sub(size);
            if chunk.len() as u64 > room {
                out.write_all(&chunk[..room as usize]).await?;
                size += room;
                truncated = true;
                continue;
            }
            size += chunk.len() as u64;
            out.write_all(&chunk).await?;
        }
        out.flush().await?;
        return Ok(Some(UploadedFile { filename, mimetype, fieldname, size, truncated }));
    }
}
