# Migrating to VelesDB 5.0.0

VelesDB 5.0.0 is versioned **MAJOR for one wire-shape change** — the
`load_working_context` envelope — that reaches every surface relaying it:
the MCP tool, the Python (`velesdb` on PyPI), Node
(`@wiscale/velesdb-memory-node`), WASM (`@wiscale/velesdb-wasm`) bindings
and the TypeScript SDK (`@wiscale/velesdb-sdk`). Everything else in the
release is additive or internal.

Read section 1 first: it is the one change that alters what your code
*receives*, not what it may call.

---

## 1. `load_working_context` returns an envelope, not the bare context

The tool (and every binding method relaying it) used to return the saved
working context directly, with `null`/`None` meaning "nothing saved". It
now returns:

```json
{ "found": true, "working": { ... }, "other_sessions": ["sprint-12", "hotfix"] }
```

- `found: false` distinguishes "nothing saved under this id" from an empty
  context — and `other_sessions` lists what IS resumable under the project,
  so a mistyped session id no longer reads as a fresh start.
- `other_sessions` is filled on a **hit** too: the wrong session resumed is
  the failure that looks like success.

*Migration*: read `.working` where you used to read the whole return value,
and branch on `.found` where you used to test for `null`:

```python
# before                                # after
ctx = mem.load_working_context(p, s)    out = mem.load_working_context(p, s)
if ctx is None: start_fresh()           if not out["found"]:
                                            # out["other_sessions"] says what exists
                                            start_fresh()
                                        ctx = out["working"]
```

The TypeScript SDK and the LangGraph toolkit detect a version skew at call
time and reject with an actionable error rather than handing back an object
whose `found` is `undefined` — but that guard is a net. Upgrade the
packages together; the floors now enforce it
(`@wiscale/velesdb-wasm: ^5.0.0` in the SDK, `velesdb>=5.0.0` in
LangGraph).

---

## 2. `WalBatcher` leaves the public Rust core API

`velesdb_core::WalBatcher` had zero call sites in the tree and is demoted
to `pub(crate)` rather than shipped as public surface users could build on
going into a major. *Migration*: no known external users. A caller who did
construct it should hold their own batching in front of `Database` writes —
or open an issue with the use case attached; the code and its tests are
intact behind the visibility change.

---

## 3. Behavior notes that are NOT breaks

- **`velesdb-memory`'s default build now carries both semantic backends**
  (`ollama` + `extract` features). Nothing changes at runtime until
  `VELESDB_MEMORY_EMBEDDER` / `VELESDB_MEMORY_EXTRACTOR` opt in — the
  default embedder is still the offline `hash`. A packager who wants the
  previous minimal binary builds with
  `--no-default-features --features mcp,context`.
- **Strict-schema graph mode returns `NodeNotFound` (`VELES-022`, HTTP 404)
  for a genuinely missing edge endpoint** — shipped in 4.x for schemaless,
  unified in the 4.0.0 train for strict mode; restated here because code
  written against pre-4.0 strict mode may still match `SchemaValidation`
  for that case.
- **New inspection surfaces** (`memory_status`, `list_memories`, the
  `export` subcommand, and their Node/Python twins) are additive; the MCP
  tool count moves from 20 to 22, which only matters to a client that
  hard-pinned the list.

---

`VelesDB 5.0.0` · Last updated: 2026-08-10 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
