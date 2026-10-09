#!/bin/sh
# Entry point for `gray plugin check <dir>` and installed-plugin spawns:
# find the built gray-acp binary and run it as a sidecar. The binary's
# dispatch loop answers plugin/manifest et al. alongside the ACP surface.
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
bin="${GRAY_ACP_BIN:-}"
if [ -z "$bin" ]; then
  for cand in "$here/target/release/gray-acp" "$here/target/debug/gray-acp"; do
    if [ -x "$cand" ]; then bin="$cand"; break; fi
  done
fi
if [ -z "$bin" ]; then
  echo "gray-acp binary not found under $here/target (run cargo build first)" >&2
  exit 1
fi
exec "$bin" sidecar
