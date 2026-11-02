//! The Hocuspocus 3 wire format (as spoken by `@hocuspocus/provider` 3.4.4): every message is
//! `varString(documentName) varUint(MessageType) …payload`, encoded with lib0's variable-length integers.
//! The sync payloads are y-protocols' (`messageYjsSyncStep1/2`, `messageYjsUpdate`).

use std::fmt;

use yrs::encoding::read::{Cursor, Read as _};
use yrs::encoding::write::Write as _;

/// `MessageType` of `@hocuspocus/server`.
pub mod msg {
    pub const SYNC: u64 = 0;
    pub const AWARENESS: u64 = 1;
    pub const AUTH: u64 = 2;
    pub const QUERY_AWARENESS: u64 = 3;
    /// Same as Sync, but won't trigger another SyncStep1.
    pub const SYNC_REPLY: u64 = 4;
    pub const STATELESS: u64 = 5;
    pub const BROADCAST_STATELESS: u64 = 6;
    pub const CLOSE: u64 = 7;
    pub const SYNC_STATUS: u64 = 8;
}

/// y-protocols sync message types.
pub mod sync {
    pub const STEP1: u64 = 0;
    pub const STEP2: u64 = 1;
    pub const UPDATE: u64 = 2;
}

/// `AuthMessageType` of `@hocuspocus/common`.
pub mod auth {
    pub const TOKEN: u64 = 0;
    pub const PERMISSION_DENIED: u64 = 1;
    pub const AUTHENTICATED: u64 = 2;
}

/// Close events of `@hocuspocus/common` (the code never travels in a CLOSE message, only the reason).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseEvent {
    pub code: u16,
    pub reason: String,
}

impl CloseEvent {
    pub fn new(code: u16, reason: impl Into<String>) -> Self {
        CloseEvent { code, reason: reason.into() }
    }
    pub fn reset_connection() -> Self {
        CloseEvent::new(4205, "Reset Connection")
    }
    pub fn unauthorized() -> Self {
        CloseEvent::new(4401, "Unauthorized")
    }
    pub fn forbidden() -> Self {
        CloseEvent::new(4403, "Forbidden")
    }
    pub fn connection_timeout() -> Self {
        CloseEvent::new(4408, "Connection Timeout")
    }
}

/// A lib0 decoding failure (`Unexpected end of array`, `Integer out of range`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub &'static str);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for DecodeError {}

const TRUNCATED: DecodeError = DecodeError("Unexpected end of array");

/// lib0 `Decoder`, on the one in yrs.
#[derive(Debug, Clone)]
pub struct Decoder<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Decoder { buf, pos: 0 }
    }

    pub fn has_content(&self) -> bool {
        self.pos < self.buf.len()
    }

    /// `readVarUint` (lib0 limits values to 53 bits).
    pub fn var_uint(&mut self) -> Result<u64, DecodeError> {
        let mut c = Cursor::new(&self.buf[self.pos..]);
        let v: u64 = c.read_var().map_err(|_| TRUNCATED)?;
        self.pos += c.next;
        if v > (1u64 << 53) - 1 {
            return Err(DecodeError("Integer out of range"));
        }
        Ok(v)
    }

    /// `readVarUint8Array`.
    pub fn var_bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = self.var_uint()? as usize;
        let end = self.pos.checked_add(len).filter(|e| *e <= self.buf.len()).ok_or(TRUNCATED)?;
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    /// `peekVarUint8Array`.
    pub fn peek_var_bytes(&self) -> Result<&'a [u8], DecodeError> {
        self.clone().var_bytes()
    }

    /// `readVarString` (lib0 decodes with a non-fatal TextDecoder: invalid UTF-8 becomes U+FFFD).
    pub fn var_string(&mut self) -> Result<String, DecodeError> {
        Ok(String::from_utf8_lossy(self.var_bytes()?).into_owned())
    }
}

/// lib0 `Encoder`, on the one in yrs.
#[derive(Debug, Clone, Default)]
pub struct Encoder {
    pub buf: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Encoder::default()
    }

    pub fn var_uint(&mut self, n: u64) -> &mut Self {
        self.buf.write_var(n);
        self
    }

    pub fn var_bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.write_buf(b);
        self
    }

    pub fn var_string(&mut self, s: &str) -> &mut Self {
        self.var_bytes(s.as_bytes())
    }

    pub fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

/// `new OutgoingMessage(documentName)`.
fn outgoing(doc: &str, ty: u64) -> Encoder {
    let mut e = Encoder::new();
    e.var_string(doc).var_uint(ty);
    e
}

/// `writeAuthenticated(readonly)`: scope `"readonly"` or `"read-write"`.
pub fn authenticated(doc: &str, read_only: bool) -> Vec<u8> {
    outgoing(doc, msg::AUTH).var_uint(auth::AUTHENTICATED).var_string(if read_only { "readonly" } else { "read-write" }).finish()
}

/// `writePermissionDenied(reason)`.
pub fn permission_denied(doc: &str, reason: &str) -> Vec<u8> {
    outgoing(doc, msg::AUTH).var_uint(auth::PERMISSION_DENIED).var_string(reason).finish()
}

/// `createSyncMessage().writeFirstSyncStepFor(document)`.
pub fn sync_step1(doc: &str, state_vector: &[u8]) -> Vec<u8> {
    outgoing(doc, msg::SYNC).var_uint(sync::STEP1).var_bytes(state_vector).finish()
}

/// The reply to a client SyncStep1 (`readSyncStep1` writes SyncStep2 into a Sync message).
pub fn sync_step2(doc: &str, update: &[u8]) -> Vec<u8> {
    outgoing(doc, msg::SYNC).var_uint(sync::STEP2).var_bytes(update).finish()
}

/// `createSyncMessage().writeUpdate(update)` (document update broadcast).
pub fn sync_update(doc: &str, update: &[u8]) -> Vec<u8> {
    outgoing(doc, msg::SYNC).var_uint(sync::UPDATE).var_bytes(update).finish()
}

/// `writeSyncStatus(updateSaved)`.
pub fn sync_status(doc: &str, saved: bool) -> Vec<u8> {
    outgoing(doc, msg::SYNC_STATUS).var_uint(u64::from(saved)).finish()
}

/// `createAwarenessUpdateMessage(awareness, clients)` with an already encoded awareness update.
pub fn awareness(doc: &str, update: &[u8]) -> Vec<u8> {
    outgoing(doc, msg::AWARENESS).var_bytes(update).finish()
}

/// `writeStateless(payload)`.
pub fn stateless(doc: &str, payload: &str) -> Vec<u8> {
    outgoing(doc, msg::STATELESS).var_string(payload).finish()
}

/// `writeCloseMessage(reason)`.
pub fn close(doc: &str, reason: &str) -> Vec<u8> {
    outgoing(doc, msg::CLOSE).var_string(reason).finish()
}

/// `writeTokenSyncRequest()`.
pub fn token_sync_request(doc: &str) -> Vec<u8> {
    outgoing(doc, msg::AUTH).var_uint(auth::TOKEN).finish()
}

/// The Yjs update carried by a client message (sync step 2 or update, as Sync or SyncReply), or None.
/// Port of `updateOf` in services/collab/src/main.ts.
pub fn update_of(raw: &[u8]) -> Option<Vec<u8>> {
    let mut d = Decoder::new(raw);
    d.var_string().ok()?;
    let ty = d.var_uint().ok()?;
    if ty != msg::SYNC && ty != msg::SYNC_REPLY {
        return None;
    }
    let step = d.var_uint().ok()?;
    if step != sync::STEP2 && step != sync::UPDATE {
        return None;
    }
    d.var_bytes().ok().map(<[u8]>::to_vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        for n in [0u64, 1, 127, 128, 300, 16_384, u32::MAX as u64, (1 << 53) - 1] {
            let mut e = Encoder::new();
            e.var_uint(n);
            let b = e.finish();
            assert_eq!(Decoder::new(&b).var_uint().unwrap(), n);
        }
        assert!(Decoder::new(&[0x80]).var_uint().is_err());
        assert!(Decoder::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]).var_uint().is_err());
    }

    #[test]
    fn update_of_extracts_step2_and_update() {
        let m = sync_update("design:x", &[1, 2, 3]);
        assert_eq!(update_of(&m), Some(vec![1, 2, 3]));
        let m = sync_step2("d", &[9]);
        assert_eq!(update_of(&m), Some(vec![9]));
        assert_eq!(update_of(&sync_step1("d", &[0])), None);
        assert_eq!(update_of(&awareness("d", &[0])), None);
        assert_eq!(update_of(&[0xff]), None);
    }

    #[test]
    fn message_layout() {
        assert_eq!(sync_status("ab", true), vec![2, b'a', b'b', 8, 1]);
        assert_eq!(authenticated("a", true), [vec![1, b'a', 2, 2, 8], b"readonly".to_vec()].concat());
        assert_eq!(close("a", "x"), vec![1, b'a', 7, 1, b'x']);
    }
}
