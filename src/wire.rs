//! Newline-delimited JSON-RPC on stdio.
//!
//! One transport serves both callers of this binary:
//! - an ACP client (Zed, t3code, another gray) sends `initialize`,
//!   `session/new`, `session/prompt`, `session/cancel`;
//! - the gray plugin host (`gray plugin check`, installed sidecars) sends
//!   `plugin/manifest`, `event/notify`, `plugin/shutdown`.
//!
//! The method namespaces never collide, so dispatch is purely by method —
//! no mode flag needed. Outgoing frames always carry `jsonrpc: "2.0"`;
//! the host tolerates the extra field (it reads `result` by key) and ACP
//! clients expect it.

use std::io::{BufRead, Write};
use std::sync::mpsc::{self, Receiver};

use serde_json::Value;

/// One inbound stdio line, classified.
pub enum Msg {
    /// `id` + `method`: a request we must answer.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// `method`, no `id`: notification, no reply.
    Notification { method: String, params: Value },
    /// Client→agent response (this agent originates no requests); ignored.
    Response,
    /// Unparseable line — tolerated, skipped.
    Bad,
    /// stdin closed.
    Eof,
}

pub fn classify(v: &Value) -> Msg {
    let method = v.get("method").and_then(Value::as_str);
    let id = v.get("id");
    match (method, id) {
        (Some(m), Some(id)) if !id.is_null() => Msg::Request {
            id: id.clone(),
            method: m.to_string(),
            params: v.get("params").cloned().unwrap_or(Value::Null),
        },
        (Some(m), _) => Msg::Notification {
            method: m.to_string(),
            params: v.get("params").cloned().unwrap_or(Value::Null),
        },
        (None, Some(_)) => Msg::Response,
        _ => Msg::Bad,
    }
}

/// `GRAY_ACP_DEBUG` set: mirror every frame to stderr.
pub fn debug() -> bool {
    std::env::var_os("GRAY_ACP_DEBUG").is_some()
}

/// Spawn the stdin reader thread. Sends every parsed line, then one
/// [`Msg::Eof`] when stdin closes.
pub fn reader() -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("acp-stdin".into())
        .spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                if debug() {
                    eprintln!("[acp<-] {}", &t[..t.len().min(300)]);
                }
                let msg = match serde_json::from_str::<Value>(t) {
                    Ok(v) => classify(&v),
                    Err(_) => Msg::Bad,
                };
                if tx.send(msg).is_err() {
                    return;
                }
            }
            let _ = tx.send(Msg::Eof);
        });
    rx
}

/// Serialize + flush one frame.
pub fn write_line(out: &mut impl Write, v: &Value) -> std::io::Result<()> {
    if debug() {
        let s = v.to_string();
        eprintln!("[acp->] {}", &s[..s.len().min(300)]);
    }
    let mut buf = serde_json::to_vec(v).map_err(std::io::Error::other)?;
    buf.push(b'\n');
    out.write_all(&buf)?;
    out.flush()
}

pub fn respond(out: &mut impl Write, id: &Value, result: Value) {
    let _ = write_line(
        out,
        &serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}),
    );
}

pub fn respond_error(out: &mut impl Write, id: &Value, code: i64, message: &str) {
    let _ = write_line(
        out,
        &serde_json::json!({"jsonrpc": "2.0", "id": id,
            "error": {"code": code, "message": message}}),
    );
}

pub fn respond_error_data(out: &mut impl Write, id: &Value, code: i64, message: &str, data: Value) {
    let _ = write_line(
        out,
        &serde_json::json!({"jsonrpc": "2.0", "id": id,
            "error": {"code": code, "message": message, "data": data}}),
    );
}

/// Fire-and-forget notification (e.g. `session/update`).
pub fn notify(out: &mut impl Write, method: &str, params: Value) {
    let _ = write_line(
        out,
        &serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params}),
    );
}

/// Emit one `session/update` notification for a session.
pub fn session_update(out: &mut impl Write, session_id: &str, update: Value) {
    notify(
        out,
        "session/update",
        serde_json::json!({"sessionId": session_id, "update": update}),
    );
}
