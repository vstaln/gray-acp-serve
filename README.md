# gray-acp-serve (`gray-acp`)

Expose **gray** as an [ACP](https://agentclientprotocol.com) agent — the
inverse of `gray-acp-bridge` (which lets gray *drive* other ACP agents).
An ACP client (Zed, t3code, or another gray) spawns `gray-acp` and talks
newline-delimited JSON-RPC on stdio; each `session/prompt` runs one
`gray -p --json` child and streams its progress back as `session/update`
notifications.

## Wire surface

| Method | Behavior |
|---|---|
| `initialize` | `protocolVersion: 1`, `agentInfo`, honest capabilities (`loadSession: false`, no image/audio/embedded-context prompts, no client MCP forwarding), `authMethods: []` |
| `authenticate` | Acknowledged (`{}`) — gray's own auth lives in `~/.gray` |
| `session/new` | `{cwd, mcpServers}` → `{sessionId}` (`acp-<uuid>`); `cwd` becomes the child cwd; `mcpServers` are accepted and ignored |
| `session/prompt` | `{sessionId, prompt:[blocks]}` → runs the turn, streams `agent_message_chunk` / `agent_thought_chunk` / `tool_call`+`tool_call_update`, replies `{stopReason, usage?}` |
| `session/cancel` | Notification → kills the running gray child; the prompt replies `stopReason: "cancelled"` |
| `session/load`, `session/resume`, `session/fork`, `session/list`, `session/set_*` | `-32601` — not advertised, not implemented in v1 |

Sidecar compat: the same loop answers `plugin/manifest` (empty
tools/hooks/commands manifest, `sidecar_args: ["sidecar"]`),
`event/notify` (ignored), and `plugin/shutdown` (clean exit), so
`gray plugin check` passes and the binary can be `gray plugin
install`-ed if desired.

## How a turn runs

`session/prompt` flattens the prompt content blocks to text (`text`
verbatim, `resource` → embedded text or URI, `resource_link` → `name
(uri)`; `image`/`audio` are dropped — the capabilities already refuse
them) and spawns:

```
gray -p <text> --json [--session <gray_session_id>] $GRAY_ACP_ARGS
```

with `GRAY_STREAM_TEXT=1`, so gray's `phase:"text"` progress rows carry
append-only `delta`s which become `agent_message_chunk`s. `tool_started`
/`tool_ran`/`tool_finished` rows map to `tool_call` / `tool_call_update`
(`in_progress` → `completed`/`failed`, `rawOutput` carried through);
`thinking`/`provider_retry`/`compacted` map to `agent_thought_chunk`s.
If no text deltas stream, the final `result.text` is sent as one chunk.

The `session_id` on gray's result row is stored and replayed as
`--session <id>` on the next `session/prompt`, so conversation history
persists across prompts within one ACP session.

Turn failures (`{"type":"error","code","message","hint"}` on the gray
wire — `auth_failed`, `rate_limited`, `context_overflow`, `session_locked`,
…) answer `session/prompt` with a JSON-RPC error carrying the message,
hint, and `retryable` flag.

## Install / run

```sh
cargo build --release          # binary: target/release/gray-acp
# Point your ACP client at the binary, e.g. Zed settings:
#   "agent_servers": { "gray": { "command": "/path/to/gray-acp" } }
gray-acp                       # serve stdio (also: `serve`, `sidecar`)
gray-acp manifest              # print the sidecar manifest JSON
gray plugin check <repo-dir>   # conformance check via ./plugin.sh
```

Environment:

- `GRAY_ACP_GRAY_BIN` — gray binary to spawn (default: `GRAY_BIN`, else
  `gray` on PATH).
- `GRAY_ACP_ARGS` — extra whitespace-split args for the child, e.g.
  `--model openai/gpt-5 --max-turns 20`.
- `GRAY_ACP_DEBUG` — mirror every wire frame + child stderr to stderr.
- `GRAY_ACP_BIN` — override the binary `plugin.sh` execs.

## Test

```sh
cargo test                                        # unit tests
# Offline (no credentials): fake gray emits the same NDJSON wire
GRAY_ACP_GRAY_BIN=$PWD/tests/fake-gray.sh \
  python3 scripts/acp-smoke.py target/debug/gray-acp "hello"
# Cancel path: slow fake + mid-turn session/cancel
GRAY_ACP_GRAY_BIN=$PWD/tests/fake-gray-slow.sh SMOKE_CANCEL=1 \
  python3 scripts/acp-smoke.py target/debug/gray-acp "hello"
# Live (needs a configured gray provider): omit GRAY_ACP_GRAY_BIN
python3 scripts/acp-smoke.py target/debug/gray-acp "Say only the word OK"
```

## Limits (v1, by design)

- **No `session/load`/resume/fork/list** — `loadSession: false`; each ACP
  session starts a fresh gray conversation (continuity via `--session`
  works *within* the process lifetime only).
- **One turn at a time per process.** Prompts arriving mid-turn are
  deferred FIFO; `session/cancel` is the only message acted on mid-turn.
  (Zed spawns one agent process per session anyway.)
- **No permission prompts.** gray runs with its own configured tool
  policy; the client is never asked (`session/request_permission` is
  never emitted). This is the documented auto-approve stance — vet your
  gray config/model trust before exposing it to a client.
- **No fs/terminal client capabilities.** gray uses its own tools against
  `cwd`; client-side `fs/*`/`terminal/*` requests are never made.
- **Text-ish prompts only.** `image`/`audio` blocks are dropped
  (capabilities refuse them); `resource`/`resource_link` degrade to text.
- **`session/prompt` errors are JSON-RPC errors**, not `stopReason`s —
  except cancellation, which is `stopReason: "cancelled"`.
- Text streaming granularity = gray's disclosed deltas (complete lines +
  flush on segment close); there is no token-level stream on the wire.

## e2e credentials needed

None for the mock/fake tests above. A *live* turn needs whatever
provider auth the host's `~/.gray` is configured with (the spawned
`gray` inherits the environment and `GRAY_HOME`), and an ACP client
(Zed/t3code) to exercise the full editor path.
