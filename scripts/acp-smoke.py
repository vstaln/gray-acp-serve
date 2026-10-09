#!/usr/bin/env python3
"""Tiny ACP client: drives gray-acp through initialize → session/new →
session/prompt and prints every frame the agent sends back, with
session/update notifications shown as a stream.

Usage:
    scripts/acp-smoke.py [path-to-gray-acp] ["prompt text"]

Environment:
    GRAY_ACP_GRAY_BIN — forwarded to the child (point at tests/fake-gray.sh
        to run without live model credentials).
    SMOKE_CANCEL=1 — send session/cancel 1s after prompt instead of
        waiting for the turn (exercises cancellation).
"""
import json
import os
import subprocess
import sys
import threading
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "GRAY_ACP_BIN", "target/debug/gray-acp")
PROMPT = sys.argv[2] if len(sys.argv) > 2 else "Say only the word OK"
CANCEL = os.environ.get("SMOKE_CANCEL") == "1"


def send(proc, msg):
    line = json.dumps(msg)
    print(f">>> {line}", flush=True)
    proc.stdin.write(line + "\n")
    proc.stdin.flush()


def main():
    env = dict(os.environ)
    proc = subprocess.Popen(
        [BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, text=True, env=env, cwd=os.getcwd(),
    )
    responses = {}
    lock = threading.Lock()
    done = threading.Event()

    def reader():
        for line in proc.stdout:
            line = line.strip()
            if not line:
                continue
            print(f"<<< {line}", flush=True)
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "id" in msg and "method" not in msg:
                with lock:
                    responses[msg["id"]] = msg
                if msg["id"] == 3:
                    done.set()
        done.set()

    threading.Thread(target=reader, daemon=True).start()

    send(proc, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": 1,
        "clientCapabilities": {},
        "clientInfo": {"name": "acp-smoke", "version": "0.0.0"},
    }})
    send(proc, {"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {
        "cwd": os.getcwd(), "mcpServers": [],
    }})

    # Wait for the session id.
    deadline = time.time() + 10
    while time.time() < deadline and 2 not in responses:
        time.sleep(0.05)
    session_id = responses.get(2, {}).get("result", {}).get("sessionId")
    if not session_id:
        print("FAIL: no sessionId from session/new")
        return 1

    send(proc, {"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {
        "sessionId": session_id,
        "prompt": [{"type": "text", "text": PROMPT}],
    }})
    if CANCEL:
        time.sleep(1)
        send(proc, {"jsonrpc": "2.0", "method": "session/cancel",
                    "params": {"sessionId": session_id}})

    done.wait(timeout=180)
    resp = responses.get(3, {})
    proc.kill()

    if "error" in resp:
        print(f"FAIL: prompt error {resp['error']}")
        return 1
    result = resp.get("result", {})
    print(f"stopReason={result.get('stopReason')} usage={result.get('usage')}")
    if CANCEL and result.get("stopReason") != "cancelled":
        print("FAIL: expected stopReason=cancelled")
        return 1
    print("PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
