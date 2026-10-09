//! The gray sidecar manifest this binary answers `plugin/manifest` with.
//! `gray-acp` is a standalone ACP server, not a channel adapter — it
//! contributes no gray tools, hooks, or commands — but speaking the
//! manifest handshake lets `gray plugin check` exercise the binary and
//! lets a user `gray plugin install` it if they want the binary managed.

use serde_json::{json, Value};

pub const PLUGIN_NAME: &str = "acp-serve";
/// Sidecar protocol version claimed to the host.
pub const PROTOCOL: &str = "1.1";

pub fn manifest() -> Value {
    json!({
        "name": PLUGIN_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": PROTOCOL,
        // The host appends these when spawning us as an installed plugin;
        // `gray-acp sidecar` runs the same dispatch loop as bare `gray-acp`.
        "sidecar_args": ["sidecar"],
        "tools": [],
        "hooks": [],
        "commands": [],
        "completion": [],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_identity_and_empty_surfaces() {
        let m = manifest();
        assert_eq!(m["name"], "acp-serve");
        assert_eq!(m["protocol"], "1.1");
        assert!(!m["version"].as_str().unwrap().is_empty());
        assert_eq!(m["tools"], json!([]));
        assert_eq!(m["hooks"], json!([]));
        assert_eq!(m["commands"], json!([]));
    }
}
