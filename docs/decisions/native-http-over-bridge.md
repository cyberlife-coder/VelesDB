# The shared daemon is reached over native HTTP, not a stdio bridge

Status: accepted

Hosts that support Streamable HTTP talk to the shared `velesdb-memory` daemon
directly. The pinned `mcp-remote` stdio bridge remains only for hosts whose
config accepts stdio alone, and those must be restarted after a long idle
period.

**Why.** The server expires an MCP session after 60 minutes of inactivity, and
SSE pings do not renew it. A request carrying an expired id is refused
immediately — measured at 0.048 s — but `mcp-remote@0.1.38` neither completes
the call nor re-initialises, so the host waits for its own timeout instead. A
fresh native session runs initialize, list and recall in 0.1–0.2 s.

**Evidence.** [PR #1807](https://github.com/cyberlife-coder/VelesDB/pull/1807),
which shipped the native wiring and the session guards.
