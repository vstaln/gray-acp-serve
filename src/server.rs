//! The dispatch loop: one stdio JSON-RPC connection serving the ACP
//! surface plus the gray sidecar handshake.
//!
//! Requests run sequentially on the main thread. While a `session/prompt`
//! turn streams, the runner keeps polling stdin so `session/cancel` for
//! the live session lands immediately; everything else that arrives
//! mid-turn is deferred and dispatched after the response (FIFO).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use serde_json::{json, Value};

use crate::manifest;
use crate::turn;
use crate::wire::{self, Msg};

/// ACP protocol version this agent speaks.
const PROTOCOL_VERSION: u64 = 1;
/// JSON-RPC "Method not found".
const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC "Invalid params".
const INVALID_PARAMS: i64 = -32602;

/// One ACP session: the client's cwd plus the gray session id to resume.
pub struct Session {
    pub cwd: PathBuf,
    pub gray_session: Option<String>,
}

/// Serve stdin/stdout until EOF or `plugin/shutdown`. Returns the exit code.
pub fn serve() -> i32 {
    let inbox = wire::reader();
    let mut out = std::io::stdout();
    let mut sessions: HashMap<String, Session> = HashMap::new();
    // Requests/notifications pulled off the wire while a turn ran. Checked
    // before the channel so mid-turn input keeps arrival order.
    let mut deferred: VecDeque<Msg> = VecDeque::new();

    loop {
        let msg = match deferred.pop_front() {
            Some(m) => Some(m),
            None => inbox.recv().ok(),
        };
        let msg = match msg {
            Some(m) => m,
            None => break, // reader thread gone
        };
        match msg {
            Msg::Eof => break,
            Msg::Bad | Msg::Response => {}
            Msg::Notification { method, params: _ } => match method.as_str() {
                // v1.1 sidecar teardown.
                "plugin/shutdown" => return 0,
                // Only meaningful inside a turn; the turn loop consumes it.
                "session/cancel" => {}
                // event/notify and any other notification: fire-and-forget.
                _ => {}
            },
            Msg::Request { id, method, params } => {
                dispatch(&mut out, &id, &method, &params, &mut sessions, &inbox, &mut deferred);
            }
        }
    }
    0
}

fn dispatch(
    out: &mut impl std::io::Write,
    id: &Value,
    method: &str,
    params: &Value,
    sessions: &mut HashMap<String, Session>,
    inbox: &Receiver<Msg>,
    deferred: &mut VecDeque<Msg>,
) {
    match method {
        // ---- gray sidecar surface (plugin check / installed plugin) ----
        "plugin/manifest" => wire::respond(out, id, manifest::manifest()),
        "tool/call" | "prompt/context" | "command/run" | "tool/before" => {
            wire::respond_error(out, id, METHOD_NOT_FOUND, "acp-serve exposes no gray plugin surfaces");
        }

        // ---- ACP lifecycle ----
        "initialize" => wire::respond(out, id, initialize_result()),
        "authenticate" => wire::respond(out, id, json!({})),
        "session/new" => session_new(out, id, params, sessions),
        "session/prompt" => session_prompt(out, id, params, sessions, inbox, deferred),

        // Honestly refused: capabilities below were never advertised.
        "session/load" | "session/resume" | "session/fork" | "session/list"
        | "session/set_mode" | "session/set_model" | "session/set_config_option" => {
            wire::respond_error(out, id, METHOD_NOT_FOUND, "not supported by gray-acp v1");
        }

        _ => wire::respond_error(out, id, METHOD_NOT_FOUND, "unknown method"),
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "agentInfo": {"name": "gray-acp", "version": env!("CARGO_PKG_VERSION")},
        "agentCapabilities": {
            // v1 scope-out, advertised honestly.
            "loadSession": false,
            "promptCapabilities": {
                "image": false,
                "audio": false,
                "embeddedContext": false,
            },
            // Client-supplied MCP servers are not forwarded to gray turns.
            "mcpCapabilities": {"http": false, "sse": false},
        },
        "authMethods": [],
    })
}

fn session_new(
    out: &mut impl std::io::Write,
    id: &Value,
    params: &Value,
    sessions: &mut HashMap<String, Session>,
) {
    let cwd = params
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    // mcpServers accepted but not forwarded (capabilities say so).
    let sid = format!("acp-{}", uuid::Uuid::new_v4());
    sessions.insert(
        sid.clone(),
        Session {
            cwd,
            gray_session: None,
        },
    );
    wire::respond(out, id, json!({"sessionId": sid}));
}

fn session_prompt(
    out: &mut impl std::io::Write,
    id: &Value,
    params: &Value,
    sessions: &mut HashMap<String, Session>,
    inbox: &Receiver<Msg>,
    deferred: &mut VecDeque<Msg>,
) {
    let sid = match params.get("sessionId").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => {
            wire::respond_error(out, id, INVALID_PARAMS, "session/prompt: missing sessionId");
            return;
        }
    };
    if !sessions.contains_key(&sid) {
        wire::respond_error(out, id, INVALID_PARAMS, "session/prompt: unknown sessionId");
        return;
    }
    let text = prompt_text(params.get("prompt").unwrap_or(&Value::Null));
    if text.trim().is_empty() {
        wire::respond(out, id, json!({"stopReason": "end_turn"}));
        return;
    }
    let session = sessions.get_mut(&sid).expect("checked above");
    let cwd = session.cwd.clone();
    let gray_session = session.gray_session.clone();
    match turn::run(out, &sid, &cwd, gray_session.as_deref(), &text, inbox, deferred) {
        Ok((result, new_gray_session)) => {
            if let Some(gs) = new_gray_session {
                if let Some(s) = sessions.get_mut(&sid) {
                    s.gray_session = Some(gs);
                }
            }
            wire::respond(out, id, result);
        }
        Err((code, message)) => {
            // A failed turn may still have produced a gray session worth
            // resuming; keep whatever the runner saw.
            wire::respond_error(out, id, code, &message);
        }
    }
}

/// Flatten ACP content blocks to plain text for `gray -p`. `text` blocks
/// pass through; `resource` contributes its embedded text (or a URI line);
/// `resource_link` becomes a `name (uri)` line; `image`/`audio` are dropped
/// (advertised capabilities already refused them).
fn prompt_text(prompt: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    let blocks: Vec<&Value> = match prompt {
        Value::Array(items) => items.iter().collect(),
        Value::String(s) => return s.clone(),
        _ => return String::new(),
    };
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    parts.push(t.to_string());
                }
            }
            Some("resource") => {
                let res = block.get("resource").cloned().unwrap_or(Value::Null);
                if let Some(t) = res.get("text").and_then(Value::as_str) {
                    parts.push(t.to_string());
                } else if let Some(u) = res.get("uri").and_then(Value::as_str) {
                    parts.push(u.to_string());
                }
            }
            Some("resource_link") => {
                let uri = block.get("uri").and_then(Value::as_str).unwrap_or("");
                let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                if !uri.is_empty() {
                    parts.push(if name.is_empty() {
                        uri.to_string()
                    } else {
                        format!("{name} ({uri})")
                    });
                }
            }
            _ => {}
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_text_flattens_blocks() {
        let p = json!([
            {"type": "text", "text": "hello"},
            {"type": "image", "data": "AAAA", "mimeType": "image/png"},
            {"type": "resource", "resource": {"uri": "file:///x", "text": "body"}},
            {"type": "resource_link", "uri": "file:///y", "name": "why"},
            {"type": "text", "text": "bye"},
        ]);
        assert_eq!(prompt_text(&p), "hello\nbody\nwhy (file:///y)\nbye");
    }

    #[test]
    fn prompt_text_accepts_bare_string() {
        assert_eq!(prompt_text(&json!("hi")), "hi");
        assert_eq!(prompt_text(&Value::Null), "");
    }
}
