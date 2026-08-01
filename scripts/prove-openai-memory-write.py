#!/usr/bin/env python3
"""Prove that a velesdb-memory daemon can WRITE against its configured embedder.

Runs a throwaway daemon over stdio on a TEMPORARY store, so the operator's own
daemon is never restarted and their real memory is never touched.

Requests are sent one at a time, each waiting for its own response: the server
answers concurrently, so a batch of lines would let `load` overtake `save` and
report `found: false` on a store that was written correctly.

Usage:
    python3 scripts/prove-openai-memory-write.py path/to/velesdb-memory

Every VELESDB_MEMORY_* variable in the environment is passed through, which is
how the embedder under test is selected.
"""

import json
import os
import subprocess
import sys
import tempfile

PROTOCOL_VERSION = "2024-11-05"
GOAL = "preuve d'ecriture sur le backend configure"


def send(proc, message):
    """Write one JSON-RPC message, then block for the reply carrying its id."""
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()
    if "id" not in message:
        return None
    while True:
        line = proc.stdout.readline()
        if not line:
            raise SystemExit("the daemon closed its output before answering")
        try:
            reply = json.loads(line)
        except json.JSONDecodeError:
            continue  # not a JSON-RPC frame
        if reply.get("id") == message["id"]:
            return reply


def call(proc, call_id, tool, arguments):
    return send(proc, {
        "jsonrpc": "2.0",
        "id": call_id,
        "method": "tools/call",
        "params": {"name": tool, "arguments": arguments},
    })


def report(label, reply):
    """Print the raw result, and say plainly whether it succeeded."""
    payload = reply.get("result") or reply.get("error")
    print(f"\n=== {label} ===")
    print(json.dumps(payload, ensure_ascii=False, indent=2))
    failed = "error" in reply or (reply.get("result") or {}).get("isError")
    print(f"--> {'ECHEC' if failed else 'OK'}")
    return not failed


def main():
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} path/to/velesdb-memory")
    binary = sys.argv[1]

    with tempfile.TemporaryDirectory() as store:
        env = dict(os.environ)
        env["VELESDB_MEMORY_PATH"] = store
        env["VELESDB_MEMORY_QUIET"] = "1"
        print("Variables VELESDB_MEMORY_* en vigueur :")
        for key in sorted(k for k in env if k.startswith("VELESDB_MEMORY_")):
            shown = "<redacted>" if "TOKEN" in key or "KEY" in key else env[key]
            print(f"  {key}={shown}")

        proc = subprocess.Popen(
            [binary],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=None,
            text=True,
            env=env,
        )
        try:
            send(proc, {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "preuve-openai", "version": "0"},
                },
            })
            send(proc, {"jsonrpc": "2.0", "method": "notifications/initialized"})

            saved = report("save_working_context", call(proc, 2, "save_working_context", {
                "project": "preuve",
                "session": "omlx",
                "working": {"goal": GOAL},
            }))
            loaded = report("load_working_context", call(proc, 3, "load_working_context", {
                "project": "preuve",
                "session": "omlx",
            }))
        finally:
            proc.stdin.close()
            proc.wait(timeout=30)

    print("\n=== VERDICT ===")
    print(f"save_working_context : {'OK' if saved else 'ECHEC'}")
    print(f"load_working_context : {'OK' if loaded else 'ECHEC'}")
    raise SystemExit(0 if saved and loaded else 1)


if __name__ == "__main__":
    main()
