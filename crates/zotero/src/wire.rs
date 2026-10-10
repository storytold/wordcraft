//! Framing and messages. Every frame is a 32-bit transaction id and a 32-bit big-endian payload
//! length, then the payload: UTF-8 JSON, or a string starting `ERR:` for a failed call.
//! Integration commands (us → Zotero) use transaction id 0 and get no reply; Zotero's calls
//! (Zotero → us) use non-zero ids that our reply echoes.

use std::io::{Read, Write};

use serde_json::{Value, json};

use crate::ZoteroError;

/// Zotero's word-processor integration port.
pub const PORT: u16 = 23116;
/// Largest payload accepted (a long bibliography's rich text is a few MB at most).
pub const MAX_FRAME: usize = 32 * 1024 * 1024;
/// Integration template version we implement.
pub const TEMPLATE_VERSION: u32 = 1;

/// One frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub txid: u32,
    pub payload: Vec<u8>,
}

/// Read a frame; `None` when the peer closed the connection cleanly between frames.
pub fn read_frame(r: &mut impl Read) -> Result<Option<Frame>, ZoteroError> {
    let mut head = [0u8; 8];
    let mut got = 0;
    while got < head.len() {
        let Some(rest) = head.get_mut(got..) else { break };
        match r.read(rest) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(ZoteroError::Protocol("connection closed inside a frame header".into())),
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(ZoteroError::Io(e.to_string())),
        }
    }
    let [t0, t1, t2, t3, l0, l1, l2, l3] = head;
    let txid = u32::from_be_bytes([t0, t1, t2, t3]);
    let len = usize::try_from(u32::from_be_bytes([l0, l1, l2, l3])).unwrap_or(usize::MAX);
    if len > MAX_FRAME {
        return Err(ZoteroError::Protocol(format!("frame of {len} bytes is over the {MAX_FRAME}-byte limit")));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).map_err(|e| ZoteroError::Io(e.to_string()))?;
    Ok(Some(Frame { txid, payload }))
}

/// Write one frame.
pub fn write_frame(w: &mut impl Write, txid: u32, payload: &[u8]) -> Result<(), ZoteroError> {
    let len = u32::try_from(payload.len()).ok().filter(|l| (*l as usize) <= MAX_FRAME);
    let Some(len) = len else { return Err(ZoteroError::Protocol("reply too large".into())) };
    let mut buf = Vec::with_capacity(8 + payload.len());
    buf.extend_from_slice(&txid.to_be_bytes());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf).and_then(|_| w.flush()).map_err(|e| ZoteroError::Io(e.to_string()))
}

/// A call from Zotero: `[method, [args…]]` (a bare `[method, arg, …]` is accepted too).
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub method: String,
    pub args: Vec<Value>,
}

impl Call {
    pub fn new(method: &str, args: Vec<Value>) -> Call {
        Call { method: method.to_string(), args }
    }

    pub fn parse(payload: &[u8]) -> Result<Call, String> {
        let v: Value = serde_json::from_slice(payload).map_err(|e| format!("not JSON: {e}"))?;
        let Value::Array(items) = v else { return Err("a call must be a JSON array".into()) };
        let mut it = items.into_iter();
        let Some(Value::String(method)) = it.next() else { return Err("a call must start with the method name".into()) };
        let rest: Vec<Value> = it.collect();
        let args = match <[Value; 1]>::try_from(rest) {
            Ok([Value::Array(a)]) => a,
            Ok([one]) => vec![one],
            Err(rest) => rest,
        };
        Ok(Call { method, args })
    }

    /// The payload Zotero would send for this call.
    pub fn to_payload(&self) -> Vec<u8> {
        serde_json::to_vec(&json!([self.method, self.args])).unwrap_or_default()
    }
}

/// The reply payload for a call's result.
pub fn encode_reply(r: &Result<Value, String>) -> Vec<u8> {
    match r {
        Ok(v) => serde_json::to_vec(v).unwrap_or_else(|_| b"null".to_vec()),
        Err(e) => format!("ERR:{e}").into_bytes(),
    }
}

/// The payload of an integration command.
pub fn command_payload(command: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"command": command, "templateVersion": TEMPLATE_VERSION})).unwrap_or_default()
}
