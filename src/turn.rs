//! One ACP `session/prompt` turn = one `gray -p --json` child.
//!
//! The child's NDJSON wire (`protocol: 1`, see crates/gray/src/print.rs):
//! - `{"type":"progress","phase":"text","segment":N,"delta","tail","done"}`
//!   — streamed assistant prose. `delta` is append-only (complete lines);
//!   `tail` is a provisional partial line the next row replaces — we forward
//!   deltas only, since a `done` delta covers the held-back tail. Enabled on
//!   the child via `GRAY_STREAM_TEXT=1`.
//! - `phase:"thinking"` (`detail`), `"tool_started"`/`"tool_ran"`/
//!   `"tool_finished"` (`tool`,`label`,`call_id`,`detail`,`output`,`error`),
//!   `"provider_retry"` (`detail`), `"compacted"`, `"generating"`,
//!   `"persisting"`, `"ask"`.
//! - `{"type":"result","text","usage",...}` on success, `{"type":"error",
//!   "code","retryable","message","hint"?}` on failure.
//!
//! Multi-turn memory comes from the session store: the `session_id` the
//! result row carries is replayed as `--session <id>` on the next prompt.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::wire::{self, Msg};

/// How long to wait for a reaped stdout-EOF'd child before killing it.
const REAP_GRACE: Duration = Duration::from_secs(5);
/// How often the turn loop polls stdin for `session/cancel`.
const POLL: Duration = Duration::from_millis(50);
/// Child stderr lines kept for diagnostics.
const STDERR_RING: usize = 40;

enum Line {
    Text(String),
    Eof,
}

/// Per-turn mutable projection state, threaded through the line mapper.
struct Turn {
    /// True once any `text` delta chunk has been sent (result.text then
    /// must NOT be re-sent — it would duplicate the streamed prose).
    streamed: bool,
    /// `session/cancel` (or client EOF) fired for this session.
    cancelled: bool,
    /// Terminal envelope: `{"type":"result"|"error", ...}`.
    outcome: Option<Value>,
    /// gray session id seen on any wire row (for `--session` resume).
    gray_session: Option<String>,
    /// toolCallIds emitted so far (call_id → toolCallId).
    calls: HashMap<String, String>,
    synth: u64,
}

/// Run one prompt turn against `cwd`, streaming `session/update` frames to
/// `out` and watching `inbox` for `session/cancel`. Returns the value for
/// the `session/prompt` result, or `(code, message)` for an error reply.
/// `deferred` collects non-cancel input that arrived mid-turn so the main
/// loop can process it after the response.
pub fn run(
    out: &mut impl std::io::Write,
    acp_sid: &str,
    cwd: &Path,
    gray_session: Option<&str>,
    text: &str,
    inbox: &Receiver<Msg>,
    deferred: &mut VecDeque<Msg>,
) -> Result<(Value, Option<String>), (i64, String)> {
    let mut cmd = Command::new(gray_bin());
    cmd.arg("-p").arg(text).arg("--json");
    if let Some(gs) = gray_session {
        cmd.arg("--session").arg(gs);
    }
    for extra in extra_args() {
        cmd.arg(extra);
    }
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GRAY_STREAM_TEXT", "1");

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Err((
                -32000,
                format!("spawn {}: {e}", gray_bin()),
            ));
        }
    };
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");

    let (line_tx, line_rx) = mpsc::channel::<Line>();
    std::thread::Builder::new()
        .name("acp-gray-stdout".into())
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(l) => {
                        if line_tx.send(Line::Text(l)).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = line_tx.send(Line::Eof);
        })
        .ok();

    let ring: Arc<Mutex<VecDeque<String>>> =
        Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_RING + 1)));
    {
        let ring = ring.clone();
        std::thread::Builder::new()
            .name("acp-gray-stderr".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    let line = match line {
                        Ok(l) => l,
                        Err(_) => break,
                    };
                    if wire::debug() {
                        eprintln!("[gray-stderr] {line}");
                    }
                    if let Ok(mut r) = ring.lock() {
                        if r.len() >= STDERR_RING {
                            r.pop_front();
                        }
                        r.push_back(line);
                    }
                }
            })
            .ok();
    }

    let mut turn = Turn {
        streamed: false,
        cancelled: false,
        outcome: None,
        gray_session: None,
        calls: HashMap::new(),
        synth: 0,
    };

    // Drain child lines until stdout EOF; between batches poll stdin so a
    // `session/cancel` lands mid-turn.
    loop {
        match line_rx.try_recv() {
            Ok(Line::Text(l)) => {
                map_line(&l, out, acp_sid, &mut turn);
                continue;
            }
            Ok(Line::Eof) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }
        match inbox.recv_timeout(POLL) {
            Ok(Msg::Notification { method, params }) if method == "session/cancel" => {
                let target = params.get("sessionId").and_then(Value::as_str);
                if target.is_none() || target == Some(acp_sid) {
                    turn.cancelled = true;
                    let _ = child.kill();
                }
            }
            Ok(Msg::Eof) => {
                // Client went away: kill the turn, drain to EOF, then the
                // main loop will exit on the dead channel.
                turn.cancelled = true;
                let _ = child.kill();
            }
            Ok(other) => deferred.push_back(other),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                turn.cancelled = true;
                let _ = child.kill();
            }
        }
    }
    // stdout EOF: reap within a grace window, else kill.
    reap(&mut child);

    finish(out, acp_sid, &mut turn, &ring)
}

/// Give a stdout-quiet child a moment to exit before reaping by force.
fn reap(child: &mut Child) {
    for _ in 0..(REAP_GRACE.as_millis() / 50) as usize {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Decide the `session/prompt` reply from the collected wire outcome.
fn finish(
    out: &mut impl std::io::Write,
    acp_sid: &str,
    turn: &mut Turn,
    ring: &Arc<Mutex<VecDeque<String>>>,
) -> Result<(Value, Option<String>), (i64, String)> {
    if turn.cancelled {
        return Ok((json!({"stopReason": "cancelled"}), turn.gray_session.take()));
    }
    match turn.outcome.take() {
        Some(v) if v.get("type").and_then(Value::as_str) == Some("result") => {
            let text = v.get("text").and_then(Value::as_str).unwrap_or("");
            if !turn.streamed && !text.is_empty() {
                send_text(out, acp_sid, "agent_message_chunk", text);
            }
            let mut result = json!({"stopReason": "end_turn"});
            if let Some(usage) = v.get("usage") {
                let u = json!({
                    "inputTokens": usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
                    "outputTokens": usage.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
                    "totalTokens": usage.get("total_tokens").and_then(Value::as_u64).unwrap_or(0),
                });
                result["usage"] = u;
            }
            Ok((result, turn.gray_session.take()))
        }
        Some(v) if v.get("type").and_then(Value::as_str) == Some("error") => {
            let message = v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("gray turn failed");
            let hint = v.get("hint").and_then(Value::as_str).unwrap_or("");
            let text = if hint.is_empty() {
                format!("gray: {message}")
            } else {
                format!("gray: {message} (hint: {hint})")
            };
            Err((
                -32000,
                text,
            ))
        }
        _ => {
            let tail = ring
                .lock()
                .map(|r| r.iter().cloned().collect::<Vec<_>>().join(" | "))
                .unwrap_or_default();
            let note = if tail.is_empty() {
                String::new()
            } else {
                format!(" [stderr: {}]", tail.chars().take(400).collect::<String>())
            };
            Err((
                -32000,
                format!("gray exited without a result envelope{note}"),
            ))
        }
    }
}

/// Parse one child stdout line into zero or more `session/update` frames.
fn map_line(line: &str, out: &mut impl std::io::Write, acp_sid: &str, turn: &mut Turn) {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return,
    };
    if let Some(sid) = v.get("session_id").and_then(Value::as_str) {
        if turn.gray_session.is_none() {
            turn.gray_session = Some(sid.to_string());
        }
    }
    match v.get("type").and_then(Value::as_str) {
        Some("progress") => map_progress(&v, out, acp_sid, turn),
        Some("result") | Some("error") => turn.outcome = Some(v),
        _ => {}
    }
}

fn map_progress(v: &Value, out: &mut impl std::io::Write, acp_sid: &str, turn: &mut Turn) {
    match v.get("phase").and_then(Value::as_str) {
        Some("text") => {
            if let Some(delta) = v.get("delta").and_then(Value::as_str) {
                if !delta.is_empty() {
                    turn.streamed = true;
                    send_text(out, acp_sid, "agent_message_chunk", delta);
                }
            }
        }
        Some("thinking") => {
            if let Some(detail) = v.get("detail").and_then(Value::as_str) {
                if !detail.is_empty() {
                    send_text(out, acp_sid, "agent_thought_chunk", detail);
                }
            }
        }
        Some("provider_retry") => {
            let detail = v.get("detail").and_then(Value::as_str).unwrap_or("retrying");
            send_text(out, acp_sid, "agent_thought_chunk", &format!("provider retry: {detail}"));
        }
        Some("compacted") => {
            send_text(out, acp_sid, "agent_thought_chunk", "(context compacted)");
        }
        Some("tool_started") => {
            let id = call_id(v, turn);
            let title = v
                .get("label")
                .or_else(|| v.get("tool"))
                .and_then(Value::as_str)
                .unwrap_or("tool");
            wire::session_update(
                out,
                acp_sid,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": id,
                    "title": title,
                    "kind": "other",
                    "status": "in_progress",
                }),
            );
        }
        Some("tool_ran") => {
            let id = call_id(v, turn);
            let label = v
                .get("label")
                .or_else(|| v.get("tool"))
                .and_then(Value::as_str)
                .unwrap_or("tool");
            let detail = v.get("detail").and_then(Value::as_str).unwrap_or("");
            let title = if detail.is_empty() {
                label.to_string()
            } else {
                format!("{label}: {detail}")
            };
            wire::session_update(
                out,
                acp_sid,
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": id,
                    "title": title,
                    "status": "in_progress",
                }),
            );
        }
        Some("tool_finished") => {
            let id = call_id(v, turn);
            let failed = v.get("error").and_then(Value::as_bool).unwrap_or(false);
            let mut update = json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "status": if failed { "failed" } else { "completed" },
            });
            if let Some(output) = v.get("output").and_then(Value::as_str) {
                if !output.is_empty() {
                    update["rawOutput"] = output.into();
                }
            }
            wire::session_update(out, acp_sid, update);
        }
        // "generating", "persisting", "ask", unknown: nothing to project.
        _ => {}
    }
}

/// The toolCallId for a wire `call_id`, synthesizing `gray-tool-N` when the
/// row lacks one. Reuses the same id across started/ran/finished rows.
fn call_id(v: &Value, turn: &mut Turn) -> String {
    let key = v
        .get("call_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(id) = turn.calls.get(&key) {
        return id.clone();
    }
    turn.synth += 1;
    let id = if key.is_empty() {
        format!("gray-tool-{}", turn.synth)
    } else {
        key.clone()
    };
    turn.calls.insert(key, id.clone());
    id
}

fn send_text(out: &mut impl std::io::Write, acp_sid: &str, kind: &str, text: &str) {
    wire::session_update(
        out,
        acp_sid,
        json!({
            "sessionUpdate": kind,
            "content": {"type": "text", "text": text},
        }),
    );
}

/// The gray binary to spawn: `GRAY_ACP_GRAY_BIN`, else `GRAY_BIN` (the
/// path gray exports to subagents), else `gray` on PATH.
fn gray_bin() -> String {
    std::env::var("GRAY_ACP_GRAY_BIN")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("GRAY_BIN").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "gray".to_string())
}

/// Extra args for the child, whitespace-split from `GRAY_ACP_ARGS`
/// (e.g. `"--model openai/gpt-5 --max-turns 20"`). No quoting support —
/// one flag per word.
fn extra_args() -> Vec<String> {
    std::env::var("GRAY_ACP_ARGS")
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_delta_maps_to_agent_chunk() {
        let mut buf: Vec<u8> = Vec::new();
        let mut turn = Turn {
            streamed: false,
            cancelled: false,
            outcome: None,
            gray_session: None,
            calls: HashMap::new(),
            synth: 0,
        };
        map_line(
            r#"{"type":"progress","phase":"text","segment":0,"delta":"hi\n","tail":"","done":false}"#,
            &mut buf,
            "s1",
            &mut turn,
        );
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("\"sessionUpdate\":\"agent_message_chunk\""));
        assert!(s.contains("\"text\":\"hi\\n\""));
        assert!(turn.streamed);
    }

    #[test]
    fn tool_rows_share_call_id() {
        let mut turn = Turn {
            streamed: false,
            cancelled: false,
            outcome: None,
            gray_session: None,
            calls: HashMap::new(),
            synth: 0,
        };
        let mut buf: Vec<u8> = Vec::new();
        map_line(
            r#"{"type":"progress","phase":"tool_started","tool":"bash","label":"Bash","call_id":"c1"}"#,
            &mut buf,
            "s1",
            &mut turn,
        );
        map_line(
            r#"{"type":"progress","phase":"tool_finished","tool":"bash","call_id":"c1","output":"ok"}"#,
            &mut buf,
            "s1",
            &mut turn,
        );
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.matches("\"toolCallId\":\"c1\"").count(), 2);
        assert!(s.contains("\"status\":\"completed\""));
    }
}
