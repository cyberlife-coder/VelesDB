#!/usr/bin/env python3
"""E2E: drive the velesdb-memory MCP server over stdio and exercise the 4 context tools."""
import json, subprocess, sys, tempfile, os

store = tempfile.mkdtemp(prefix="veles-mcp-e2e-")
proc = subprocess.Popen(
    ["./target/debug/velesdb-memory"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
    env={**os.environ, "VELESDB_MEMORY_PATH": store}, text=True,
)
rid = [0]
def rpc(method, params=None, notify=False):
    msg = {"jsonrpc": "2.0", "method": method}
    if params is not None: msg["params"] = params
    if not notify:
        rid[0] += 1
        msg["id"] = rid[0]
    proc.stdin.write(json.dumps(msg) + "\n"); proc.stdin.flush()
    if notify: return None
    while True:
        line = proc.stdout.readline()
        if not line: sys.exit("server closed stdout")
        resp = json.loads(line)
        if resp.get("id") == rid[0]:
            if "error" in resp: return {"__error__": resp["error"]}
            return resp["result"]

def call(tool, args):
    result = rpc("tools/call", {"name": tool, "arguments": args})
    if "__error__" in result: return result
    return json.loads(result["content"][0]["text"]) if result.get("content") else result.get("structuredContent")

rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}})
rpc("notifications/initialized", notify=True)

tools = [t["name"] for t in rpc("tools/list")["tools"]]
expected = {"compile_context", "context_savings", "explain_compilation", "retrieve_context_source"}
assert expected <= set(tools), f"missing context tools: {expected - set(tools)} in {tools}"

req = {"query": "deploy pipeline", "token_budget": 10000, "project": "veles",
       "fragments": [{"content": "The deploy pipeline runs clippy before tests."},
                      {"content": "The deploy pipeline runs clippy before tests."},
                      {"content": "Never restart the primary during a rebalance."}]}
out = call("compile_context", req)
assert "Never restart" in out["content"], out
assert len(out["decisions"]) == 3
drop = next(d for d in out["decisions"] if d["action"] == "drop")
assert drop["rule_id"] == "drop.duplicate"
assert out["insights"]["tokens_saved"] > 0
handle = out["sources"][0]["handle"]
assert handle.startswith("ctx://source/")

src = call("retrieve_context_source", {"handle": handle})
assert src["content"] in {f["content"] for f in req["fragments"]}, src

explain = call("explain_compilation", {"request": req, "fragment_id": drop["fragment_id"]})
assert explain["rule_id"] in {"drop.duplicate", "preserve.default"}, explain

savings = call("context_savings", {"project": "veles"})
assert savings["events"] == 1 and savings["tokens_saved"] > 0, savings

err = call("compile_context", {"query": "x", "token_budget": 0, "fragments": [{"content": "y"}]})
assert "__error__" in err and err["__error__"]["code"] == -32602, err

proc.terminate()
print("MCP E2E OK — 4 tools exercised over real stdio: list, compile (dedup+insights+handles), retrieve round-trip, explain, savings, error taxonomy")
