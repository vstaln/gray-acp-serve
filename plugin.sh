#!/bin/sh
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
sys="$(uname -s)"
case "$sys" in
MINGW* | MSYS* | CYGWIN*) rel="x86_64-pc-windows-msvc/gray-acp.exe" ;;
Darwin*)
    case "$(uname -m)" in
    arm64) rel="aarch64-apple-darwin/gray-acp" ;;
    *) rel="x86_64-apple-darwin/gray-acp" ;;
    esac ;;
*)
    case "$(uname -m)" in
    aarch64 | arm64) rel="aarch64-unknown-linux-musl/gray-acp" ;;
    *) rel="x86_64-unknown-linux-musl/gray-acp" ;;
    esac ;;
esac
if [ -x "$here/$rel" ]; then exec "$here/$rel" sidecar "$@"; fi
bin="${{GRAY_ACP_BIN:-}}"
if [ -z "$bin" ]; then
    for cand in "$here/target/release/gray-acp.exe" "$here/target/release/gray-acp" "$here/target/debug/gray-acp.exe" "$here/target/debug/gray-acp"; do
        if [ -x "$cand" ]; then bin="$cand"; break; fi
    done
fi
if [ -z "$bin" ]; then
    echo "gray-acp binary not found under $here/target (run cargo build first)" >&2
    exit 1
fi
exec "$bin" sidecar "$@"
