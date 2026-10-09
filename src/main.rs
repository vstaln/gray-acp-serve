//! `gray-acp` — ACP agent server over stdio.
//!
//! Usage:
//!   gray-acp            serve ACP + sidecar dispatch on stdio (default)
//!   gray-acp serve      same as bare
//!   gray-acp sidecar    same (manifest `sidecar_args` entry point)
//!   gray-acp manifest   print the gray plugin manifest JSON
//!   gray-acp --help | --version

fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        None | Some("serve") | Some("sidecar") => {
            std::process::exit(gray_acp_serve::server::serve());
        }
        Some("manifest") => {
            println!("{}", gray_acp_serve::manifest::manifest());
        }
        Some("--version") | Some("-V") => {
            println!("gray-acp {}", env!("CARGO_PKG_VERSION"));
        }
        Some("--help") | Some("-h") => {
            println!(
                "gray-acp — expose gray as an ACP (Agent Client Protocol) agent.\n\
                 \n\
                 Spawned by an ACP client (Zed, t3code, another gray) with no\n\
                 arguments it speaks newline-delimited JSON-RPC on stdio:\n\
                 initialize, session/new, session/prompt (streams session/update),\n\
                 session/cancel. Each prompt runs `gray -p --json` as a child.\n\
                 \n\
                 It also answers the gray sidecar handshake (plugin/manifest,\n\
                 plugin/shutdown) so `gray plugin check <dir>` passes.\n\
                 \n\
                 Subcommands: serve (default), sidecar, manifest.\n\
                 Env: GRAY_ACP_GRAY_BIN (gray binary), GRAY_ACP_ARGS (extra\n\
                 args for the child), GRAY_ACP_DEBUG (frame log on stderr)."
            );
        }
        Some(other) => {
            eprintln!("gray-acp: unknown argument {other:?} (try --help)");
            std::process::exit(2);
        }
    }
}
