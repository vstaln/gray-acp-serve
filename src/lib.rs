//! `gray-acp` — gray exposed as an ACP (Agent Client Protocol) agent.
//!
//! ACP clients (Zed, t3code, another gray's acp-bridge) spawn this binary
//! and drive it with newline-delimited JSON-RPC over stdio:
//! `initialize` → `session/new` → `session/prompt` (+ `session/update`
//! streaming) → `session/cancel`. Each prompt runs as a `gray -p --json`
//! child whose progress rows are projected to ACP updates.
//!
//! The same loop answers the gray sidecar handshake (`plugin/manifest`,
//! `plugin/shutdown`) so `gray plugin check` can exercise the binary.

pub mod manifest;
pub mod server;
pub mod turn;
pub mod wire;
