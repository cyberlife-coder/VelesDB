#!/usr/bin/env python3
"""E2E: drive the velesdb-memory MCP server over stdio and exercise the 6 context tools.

Two separate server processes share one store path: the second launch proves
save_working_context / load_working_context round-trip ACROSS processes — the
inter-session resumption the tools exist for.
"""
import json, subprocess, sys, tempfile, os


class Server:
    """One velesdb-memory MCP server over stdio, bound to `store`."""

    def __init__(self, store):
        self.proc = subprocess.Popen(
            ["./target/debug/velesdb-memory"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env={**os.environ, "VELESDB_MEMORY_PATH": store}, text=True,
        )
        self.rid = 0
        self.rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                                "clientInfo": {"name": "e2e", "version": "0"}})
        self.rpc("notifications/initialized", notify=True)

    def rpc(self, method, params=None, notify=False):
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None: msg["params"] = params
        if not notify:
            self.rid += 1
            msg["id"] = self.rid
        self.proc.stdin.write(json.dumps(msg) + "\n"); self.proc.stdin.flush()
        if notify: return None
        while True:
            line = self.proc.stdout.readline()
            if not line: sys.exit("server closed stdout")
            resp = json.loads(line)
            if resp.get("id") == self.rid:
                if "error" in resp: return {"__error__": resp["error"]}
                return resp["result"]

    def call(self, tool, args):
        result = self.rpc("tools/call", {"name": tool, "arguments": args})
        if "__error__" in result: return result
        return json.loads(result["content"][0]["text"]) if result.get("content") else result.get("structuredContent")

    def terminate(self):
        # Wait for the process to actually exit: the next server in this
        # script reopens the SAME store, and racing the shutdown would hit
        # the previous process's store lock.
        self.proc.terminate()
        self.proc.wait(timeout=10)


store = tempfile.mkdtemp(prefix="veles-mcp-e2e-")
srv = Server(store)

tools = [t["name"] for t in srv.rpc("tools/list")["tools"]]
expected = {"compile_context", "context_savings", "explain_compilation",
            "retrieve_context_source", "save_working_context", "load_working_context"}
assert expected <= set(tools), f"missing context tools: {expected - set(tools)} in {tools}"

req = {"query": "deploy pipeline", "token_budget": 10000, "project": "veles",
       "fragments": [{"content": "The deploy pipeline runs clippy before tests."},
                      {"content": "The deploy pipeline runs clippy before tests."},
                      {"content": "Never restart the primary during a rebalance."}]}
out = srv.call("compile_context", req)
assert "Never restart" in out["content"], out
assert len(out["decisions"]) == 3
drop = next(d for d in out["decisions"] if d["action"] == "drop")
assert drop["rule_id"] == "drop.duplicate"
assert out["insights"]["tokens_saved"] > 0
handle = out["sources"][0]["handle"]
assert handle.startswith("ctx://source/")

src = srv.call("retrieve_context_source", {"handle": handle})
assert src["content"] in {f["content"] for f in req["fragments"]}, src

explain = srv.call("explain_compilation", {"request": req, "fragment_id": drop["fragment_id"]})
assert explain["rule_id"] in {"drop.duplicate", "preserve.default"}, explain

savings = srv.call("context_savings", {"project": "veles"})
assert savings["events"] == 1 and savings["tokens_saved"] > 0, savings

err = srv.call("compile_context", {"query": "x", "token_budget": 0, "fragments": [{"content": "y"}]})
assert "__error__" in err and err["__error__"]["code"] == -32602, err

# --- working context: save in THIS process, load in a FRESH one -------------

fresh = srv.call("load_working_context", {"project": "veles", "session": "e2e-session"})
assert fresh["working"] is None, f"nothing saved yet, expected null: {fresh}"

working = {"goal": "prove inter-session resumption over stdio",
           "active_constraints": [{"text": "never merge without green gates"}],
           "verified_facts": [{"text": "the 4 compiler tools already pass this script"}],
           "pending_actions": ["load this back from a separate server process"]}
saved = srv.call("save_working_context",
                 {"project": "veles", "session": "e2e-session", "working": working})
assert saved["id"] > 0, saved
srv.terminate()

# Second, separate server process on the same store: the next session resumes.
srv2 = Server(store)
loaded = srv2.call("load_working_context", {"project": "veles", "session": "e2e-session"})
resumed = loaded["working"]
assert resumed is not None, "the saved working context must survive the process boundary"
assert resumed["goal"] == working["goal"], resumed
assert [f["text"] for f in resumed["verified_facts"]] == [f["text"] for f in working["verified_facts"]], resumed
assert resumed["pending_actions"] == working["pending_actions"], resumed
srv2.terminate()

print("MCP E2E OK — 6 tools exercised over real stdio: list, compile (dedup+insights+handles), "
      "retrieve round-trip, explain, savings, error taxonomy, and save/load_working_context "
      "round-tripping ACROSS two separate server processes")
